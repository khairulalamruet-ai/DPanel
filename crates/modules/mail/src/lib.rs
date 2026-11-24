//! Email of a hosting account (T13.03 - T13.07, T13.10 - T13.12).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/email/mailboxes` | the owner; the `email` feature |
//! | DELETE | `/api/v1/email/mailboxes/{id}` | the owner |
//! | POST | `/api/v1/email/mailboxes/{id}/password` | the owner |
//! | POST | `/api/v1/email/mailboxes/{id}/webmail` | the owner; the `webmail` feature |
//! | GET, POST | `/api/v1/email/forwarders`, DELETE `.../{id}` | the owner |
//! | GET, POST | `/api/v1/email/autoresponders`, DELETE `.../{id}` | the owner |
//! | GET | `/api/v1/email/config` | the owner |
//! | GET | `/api/v1/admin/email/queue`, POST `.../queue/flush`, DELETE `.../queue/{id}` | `admin.email.*` |
//! | GET | `/api/v1/admin/email/deliverability` | `admin.email.read` |
//! | GET | `/api/v1/admin/email/domains`, POST `.../{id}/hold`, `.../{id}/release` | `admin.email.*` |
//! | GET, PUT | `/api/v1/admin/email/servers`, `.../servers/{node}` | `admin.email.*` |
//!
//! A person makes mail by making a mailbox in one of the account's domains: the domain gets
//! its mail domain row, and from then on [`tick::tick`] tells the node's mail server what it
//! does not have yet (every change raises a version counter; the tick sends the difference)
//! and puts the DNS records the mail server asks for (MX, SPF, DKIM, DMARC) into the domain's
//! zone. Webmail opens with a one-time link (the node signs the client in as the mailbox; the
//! panel never knows the mailbox's password). Mailbox passwords are sealed with the panel's master key until the node has them.
//! A domain whose outgoing queue grows past the limit is held (T13.07).

pub mod tick;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_database::{
    accounts::DbAccountKind,
    domains,
    mail::{self as store, MailDomain, Mailbox, MakeError},
    operations::{self, NewOperation},
    plans::{self, Plan},
    sqlx::PgPool,
    tenants, users,
};
use builtpanel_domain::{Fqdn, databases::check_password};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::{MailQueueItem, Operation};
use builtpanel_rbac::{
    Principal, Require,
    permissions::{EmailManage, EmailRead},
};
use builtpanel_session::{SessionStore, rfc3339};
use builtpanel_settings::{Scope, SettingsRegistry};
use serde::{Deserialize, Serialize};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/mail/module.toml");
/// The module's settings schema.
pub const SETTINGS_SCHEMA: &str = include_str!("../../../../modules/mail/settings.schema.json");

/// What the module's handlers share.
#[derive(Clone)]
pub struct MailState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The checked settings schemas of all modules.
    pub settings: Arc<SettingsRegistry>,
    /// Seals the passwords of mailboxes until the node has them.
    pub master_key: MasterKey,
}

impl FromRef<MailState> for PgPool {
    fn from_ref(s: &MailState) -> Self {
        s.db.clone()
    }
}

impl FromRef<MailState> for SessionStore {
    fn from_ref(s: &MailState) -> Self {
        s.sessions.clone()
    }
}

/// Routes for hosting accounts.
pub fn client_router() -> Router<MailState> {
    Router::new()
        .route(
            "/api/v1/email/mailboxes",
            get(list_mailboxes).post(create_mailbox),
        )
        .route("/api/v1/email/mailboxes/{id}", delete(delete_mailbox))
        .route(
            "/api/v1/email/mailboxes/{id}/password",
            post(change_password),
        )
        .route("/api/v1/email/mailboxes/{id}/webmail", post(open_webmail))
        .route(
            "/api/v1/email/forwarders",
            get(list_forwarders).post(create_forwarder),
        )
        .route("/api/v1/email/forwarders/{id}", delete(delete_forwarder))
        .route(
            "/api/v1/email/autoresponders",
            get(list_autoresponders).post(create_autoresponder),
        )
        .route(
            "/api/v1/email/autoresponders/{id}",
            delete(delete_autoresponder),
        )
        .route("/api/v1/email/config", get(client_config))
}

/// Routes for administrators.
pub fn admin_router() -> Router<MailState> {
    Router::new()
        .route("/api/v1/admin/email/queue", get(queue))
        .route("/api/v1/admin/email/queue/flush", post(flush_queue))
        .route("/api/v1/admin/email/queue/{id}", delete(delete_queued))
        .route("/api/v1/admin/email/deliverability", get(deliverability))
        .route("/api/v1/admin/email/domains", get(list_domains))
        .route("/api/v1/admin/email/domains/{id}/hold", post(hold_domain))
        .route(
            "/api/v1/admin/email/domains/{id}/release",
            post(release_domain),
        )
        .route("/api/v1/admin/email/servers", get(list_servers))
        .route("/api/v1/admin/email/servers/{node_id}", put(set_server))
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

fn field(name: &'static str, code: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(name, code, message)])
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// The module's settings as they stand.
pub struct Rules {
    /// The mail server's name (empty: not set).
    pub hostname: String,
    /// The name webmail is served under (empty: not set).
    pub webmail_host: String,
    /// The quota of a mailbox made without one, in MB.
    pub default_quota_mb: i32,
    /// Queued messages that hold a domain.
    pub abuse_queue_threshold: i64,
}

/// Reads the module's settings.
///
/// # Errors
/// Database errors.
pub async fn rules(registry: &SettingsRegistry, db: &PgPool) -> Result<Rules, AppError> {
    let mut conn = db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(registry, &mut conn, "mail", Scope::Global)
        .await
        .map_err(internal)?;
    let text = |key: &str| {
        values
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    let int = |key: &str, default: i64| {
        values
            .get(key)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(default)
    };
    Ok(Rules {
        hostname: text("hostname").to_ascii_lowercase(),
        webmail_host: text("webmail_host").to_ascii_lowercase(),
        default_quota_mb: i32::try_from(int("default_quota_mb", 1024)).unwrap_or(1024),
        abuse_queue_threshold: int("abuse_queue_threshold", 500),
    })
}

fn plan_limit(plan: &Plan, key: &str, default: i64) -> Option<i64> {
    let value = plan
        .limits
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(default);
    (value >= 0).then_some(value)
}

/// The plan's outgoing limit an hour.
pub fn per_hour_of(plan: &Plan) -> Option<i32> {
    plan_limit(plan, "mail_per_hour", 200).and_then(|v| i32::try_from(v.max(1)).ok())
}

struct Hosting {
    tenant: tenants::Tenant,
    plan: Plan,
}

async fn hosting(state: &MailState, who: &Principal) -> Result<Hosting, AppError> {
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
    if !plans::includes_feature(&state.db, plan.id, "email")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Email is not included in this plan.",
        ));
    }
    Ok(Hosting { tenant, plan })
}

async fn audit(
    state: &MailState,
    action: &'static str,
    who: &Principal,
    tenant: Option<Uuid>,
    kind: &'static str,
    id: Uuid,
) -> Result<(), AppError> {
    let mut entry = AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target(kind, id);
    if let Some(tenant) = tenant {
        entry = entry.tenant(tenant);
    }
    builtpanel_audit::insert(&state.db, &entry)
        .await
        .map_err(internal)
}

fn local_ok(local: &str) -> bool {
    let bytes = local.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(b))
        && bytes
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes
            .last()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// A person's address elsewhere: one `@`, a name with a dot after it, nothing odd.
fn external_address_ok(address: &str) -> bool {
    let Some((local, host)) = address.split_once('@') else {
        return false;
    };
    address.len() <= 254
        && !local.is_empty()
        && local.len() <= 64
        && local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-=".contains(&b))
        && host.contains('.')
        && Fqdn::parse(host).is_ok()
}

/// The domain of the account with this name.
async fn own_domain(
    state: &MailState,
    h: &Hosting,
    name: &str,
) -> Result<domains::Domain, AppError> {
    let fqdn =
        Fqdn::parse(name).map_err(|_| field("domain", "invalid", "This is not a domain."))?;
    domains::find(&state.db, &fqdn)
        .await
        .map_err(internal)?
        .filter(|d| d.tenant_id == h.tenant.id)
        .ok_or_else(|| {
            field(
                "domain",
                "unknown_domain",
                "This account has no such domain.",
            )
        })
}

/// The mail domain of one of the account's domains (made when it has none).
async fn mail_domain_for(
    state: &MailState,
    h: &Hosting,
    name: &str,
) -> Result<MailDomain, AppError> {
    let d = own_domain(state, h, name).await?;
    store::ensure_domain(
        &state.db,
        h.tenant.id,
        d.id,
        &d.fqdn,
        h.tenant.node_id,
        per_hour_of(&h.plan),
    )
    .await
    .map_err(
        |e| match e.as_database_error().and_then(|d| d.code()).as_deref() {
            // The same name is still being removed from the mail server.
            Some("23505") => AppError::conflict(
                "domain_busy",
                "Mail of this domain is still being removed; try again in a moment.",
            ),
            _ => internal(e),
        },
    )
}

/// A mail domain the account has (not made).
async fn existing_mail_domain(
    state: &MailState,
    h: &Hosting,
    name: &str,
) -> Result<MailDomain, AppError> {
    let d = own_domain(state, h, name).await?;
    store::domain_of(&state.db, h.tenant.id, &d.fqdn)
        .await
        .map_err(internal)?
        .ok_or_else(|| field("domain", "no_mail", "Mail is not set up for this domain."))
}

fn created(secs: i64) -> String {
    rfc3339(secs)
}

// ------------------------------------------------------------------------------ mailboxes

/// A mailbox.
#[derive(Debug, Serialize, ToSchema)]
pub struct MailboxView {
    /// Id.
    pub id: Uuid,
    /// The whole address.
    pub email: String,
    /// The part before the `@`.
    pub username: String,
    /// The domain.
    pub domain: String,
    /// Quota in megabytes.
    pub quota_mb: i32,
    /// Bytes stored.
    pub used_bytes: i64,
    /// When it was made.
    pub created_at: String,
    /// Where webmail is (empty when there is none).
    pub webmail_url: String,
    /// `pending` (the mail server is being told), `active` or `error`.
    pub status: String,
    /// Why the mail server refused the last change.
    pub error: Option<String>,
}

/// Where webmail is, as a web address (empty when it is not set up).
fn webmail_url(rules: &Rules) -> String {
    if rules.webmail_host.is_empty() {
        String::new()
    } else {
        format!("https://{}/", rules.webmail_host)
    }
}

fn mailbox_view(m: Mailbox, webmail_url: &str) -> MailboxView {
    MailboxView {
        id: m.id,
        email: format!("{}@{}", m.local_part, m.fqdn),
        username: m.local_part,
        domain: m.fqdn,
        quota_mb: m.quota_mb,
        used_bytes: m.used_bytes,
        created_at: created(m.created_at),
        webmail_url: webmail_url.to_owned(),
        status: m.state,
        error: m.last_error,
    }
}

/// What to list: one domain or all.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DomainQuery {
    /// Only this domain.
    pub domain: Option<String>,
}

/// The mailboxes of the account.
#[utoipa::path(
    get,
    path = "/api/v1/email/mailboxes",
    tag = "email",
    operation_id = "clientListMailboxes",
    security(("session" = [])),
    params(DomainQuery),
    responses(
        (status = 200, description = "The mailboxes", body = Vec<MailboxView>),
        (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_mailboxes(
    State(state): State<MailState>,
    who: Principal,
    Query(q): Query<DomainQuery>,
) -> Result<Json<Vec<MailboxView>>, AppError> {
    let h = hosting(&state, &who).await?;
    let rules = rules(&state.settings, &state.db).await?;
    let domain = q.domain.as_deref().map(str::to_ascii_lowercase);
    let items = store::mailboxes_of_tenant(&state.db, h.tenant.id, domain.as_deref())
        .await
        .map_err(internal)?
        .into_iter()
        .map(|m| mailbox_view(m, &webmail_url(&rules)))
        .collect();
    Ok(Json(items))
}

/// A mailbox to make.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MailboxRequest {
    /// The part before the `@` (lower-case letters, digits and `. _ -`).
    #[schema(example = "info")]
    pub username: String,
    /// One of the account's domains.
    #[schema(example = "example.com")]
    pub domain: String,
    /// The password (12 to 128 printable characters, three kinds of them).
    pub password: String,
    /// Quota in megabytes (the panel's default when left out).
    #[serde(default)]
    pub quota_mb: Option<i32>,
}

/// Make a mailbox.
#[utoipa::path(
    post,
    path = "/api/v1/email/mailboxes",
    tag = "email",
    operation_id = "clientCreateMailbox",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = MailboxRequest,
    responses(
        (status = 201, description = "Made", body = MailboxView),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`address_taken`, `limit_reached` or `not_configured`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The name, domain, password or size are not valid", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_mailbox(
    State(state): State<MailState>,
    who: Principal,
    Json(req): Json<MailboxRequest>,
) -> Result<(StatusCode, Json<MailboxView>), AppError> {
    let h = hosting(&state, &who).await?;
    let rules = rules(&state.settings, &state.db).await?;
    if rules.hostname.is_empty()
        && store::server(&state.db, h.tenant.node_id)
            .await
            .map_err(internal)?
            .is_none()
    {
        return Err(AppError::conflict(
            "not_configured",
            "The panel's mail server has no name yet; ask the administrator.",
        ));
    }
    let local = req.username.trim().to_ascii_lowercase();
    if !local_ok(&local) {
        return Err(field(
            "username",
            "invalid",
            "Use lower-case letters, digits and . _ - (1 to 64 characters, starting and ending with a letter or digit).",
        ));
    }
    check_password(&req.password).map_err(|e| field("password", "weak", &e.to_string()))?;
    let quota_mb = req.quota_mb.unwrap_or(rules.default_quota_mb);
    if !(1..=1_048_576).contains(&quota_mb) {
        return Err(field(
            "quota_mb",
            "out_of_range",
            "The size must be 1 MB to 1 TB.",
        ));
    }
    if let Some(limit) = plan_limit(&h.plan, "max_email_accounts", 10)
        && store::count_mailboxes(&state.db, h.tenant.id)
            .await
            .map_err(internal)?
            >= limit
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("The plan allows {limit} mailboxes."),
        ));
    }
    let md = mail_domain_for(&state, &h, &req.domain).await?;
    if md.state == "removing" {
        return Err(AppError::conflict(
            "domain_busy",
            "Mail of this domain is being removed.",
        ));
    }
    let forwarders = store::forwarders_of_domain(&state.db, md.id)
        .await
        .map_err(internal)?;
    if forwarders.iter().any(|f| f.local_part == local) {
        return Err(AppError::conflict(
            "address_taken",
            "A forwarder has this address.",
        ));
    }
    let id = Uuid::new_v4();
    let sealed = state
        .master_key
        .seal(&store::password_context(id), req.password.as_bytes())
        .map_err(internal)?;
    store::create_mailbox(&state.db, id, h.tenant.id, md.id, &local, quota_mb, &sealed)
        .await
        .map_err(|e| match e {
            MakeError::Taken => AppError::conflict("address_taken", "This address exists already."),
            MakeError::Database(e) => internal(e),
        })?;
    audit(
        &state,
        "mailbox.created",
        &who,
        Some(h.tenant.id),
        "mailbox",
        id,
    )
    .await?;
    let made = store::mailbox(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("mailbox"))?;
    Ok((
        StatusCode::CREATED,
        Json(mailbox_view(made, &webmail_url(&rules))),
    ))
}

/// Remove a mailbox with its mail.
#[utoipa::path(
    delete,
    path = "/api/v1/email/mailboxes/{id}",
    tag = "email",
    operation_id = "clientDeleteMailbox",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Mailbox id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 204, description = "Being removed"),
        (status = 404, description = "No such mailbox", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_mailbox(
    State(state): State<MailState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = hosting(&state, &who).await?;
    store::mailbox(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("mailbox"))?;
    store::mailbox_removing(&state.db, id)
        .await
        .map_err(internal)?;
    audit(
        &state,
        "mailbox.deleted",
        &who,
        Some(h.tenant.id),
        "mailbox",
        id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A new password.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PasswordRequest {
    /// The password (12 to 128 printable characters, three kinds of them).
    pub password: String,
}

/// What a change that needs no answer says.
#[derive(Debug, Serialize, ToSchema)]
pub struct Done {
    /// Always true.
    pub success: bool,
}

/// Change a mailbox's password.
#[utoipa::path(
    post,
    path = "/api/v1/email/mailboxes/{id}/password",
    tag = "email",
    operation_id = "clientChangeMailboxPassword",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Mailbox id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = PasswordRequest,
    responses(
        (status = 200, description = "Changed", body = Done),
        (status = 404, description = "No such mailbox", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The password is not strong enough", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn change_password(
    State(state): State<MailState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<PasswordRequest>,
) -> Result<Json<Done>, AppError> {
    let h = hosting(&state, &who).await?;
    check_password(&req.password).map_err(|e| field("password", "weak", &e.to_string()))?;
    store::mailbox(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .filter(|m| m.state != "removing")
        .ok_or_else(|| AppError::not_found("mailbox"))?;
    let sealed = state
        .master_key
        .seal(&store::password_context(id), req.password.as_bytes())
        .map_err(internal)?;
    store::set_password(&state.db, id, &sealed)
        .await
        .map_err(internal)?;
    audit(
        &state,
        "mailbox.password_changed",
        &who,
        Some(h.tenant.id),
        "mailbox",
        id,
    )
    .await?;
    Ok(Json(Done { success: true }))
}

// ------------------------------------------------------------------------------- webmail

/// How long the request waits for the node (the first one starts webmail).
const WEBMAIL_WAIT: std::time::Duration = std::time::Duration::from_secs(40);
/// Most links an account may ask for in ten minutes.
const MAX_LINKS_PER_TEN_MINUTES: i64 = 20;

/// A link into webmail.
#[derive(Debug, Serialize, ToSchema)]
pub struct WebmailLink {
    /// Open this address (it works once).
    #[schema(example = "https://webmail.example.com/?_dhsignon=...")]
    pub url: String,
    /// Until when the link can be opened (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub expires_at: String,
}

async fn queue_for(
    state: &MailState,
    h: &Hosting,
    op: &Operation,
    key: &str,
    who: &Principal,
) -> Result<Uuid, AppError> {
    let payload = serde_json::to_value(op).map_err(internal)?;
    let queued = operations::enqueue(
        &state.db,
        &NewOperation {
            node_id: h.tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(h.tenant.id),
            payload: &payload,
            idempotency_key: key,
            created_by: Some(who.account_id()),
            ttl_secs: 600,
            max_attempts: 3,
        },
    )
    .await
    .map_err(internal)?;
    Ok(queued.operation().id)
}

/// Open webmail as a mailbox, with one click.
#[utoipa::path(
    post,
    path = "/api/v1/email/mailboxes/{id}/webmail",
    tag = "email",
    operation_id = "clientOpenWebmail",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Mailbox id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 200, description = "The link", body = WebmailLink),
        (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such mailbox", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`not_configured` or `mailbox_not_ready`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 429, description = "`too_many_signins`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "The node is busy or webmail could not start", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn open_webmail(
    State(state): State<MailState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<WebmailLink>, AppError> {
    use sha2::{Digest as _, Sha256};
    let h = hosting(&state, &who).await?;
    if !plans::includes_feature(&state.db, h.plan.id, "webmail")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Webmail is not included in this plan.",
        ));
    }
    let mailbox = store::mailbox(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .filter(|m| m.state != "removing")
        .ok_or_else(|| AppError::not_found("mailbox"))?;
    if mailbox.state != "active" {
        return Err(AppError::conflict(
            "mailbox_not_ready",
            "The mailbox is not ready yet; try again in a moment.",
        ));
    }
    let host = rules(&state.settings, &state.db).await?.webmail_host;
    if host.is_empty() {
        return Err(AppError::conflict(
            "not_configured",
            "Webmail has no name yet; ask the administrator.",
        ));
    }
    let recent: i64 = builtpanel_database::sqlx::query_scalar(
        "SELECT count(*) FROM operations
         WHERE tenant_id = $1 AND kind = 'open_webmail' AND created_at > now() - interval '10 minutes'",
    )
    .bind(h.tenant.id)
    .fetch_one(&state.db)
    .await
    .map_err(internal)?;
    if recent >= MAX_LINKS_PER_TEN_MINUTES {
        return Err(AppError::too_many_requests(
            "too_many_signins",
            "Too many webmail sign-ins; wait a few minutes.",
            std::time::Duration::from_secs(300),
        ));
    }
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let token_hash = hex(&Sha256::digest(token.as_bytes()));
    let host_key = hex(&Sha256::digest(host.as_bytes())[..8]);
    // The route comes first (one operation per name, however many sign-ins).
    let route = queue_for(
        &state,
        &h,
        &Operation::SetWebmailHost {
            host: Some(host.clone()),
        },
        &format!("webmailhost:{host_key}"),
        &who,
    )
    .await?;
    let open = queue_for(
        &state,
        &h,
        &Operation::OpenWebmail {
            address: format!("{}@{}", mailbox.local_part, mailbox.fqdn),
            token_hash,
        },
        &format!("webmail:{}", Uuid::new_v4().simple()),
        &who,
    )
    .await?;
    audit(
        &state,
        "mailbox.webmail",
        &who,
        Some(h.tenant.id),
        "mailbox",
        id,
    )
    .await?;
    // The link is worth giving only once the node has made the sign-in.
    let started = std::time::Instant::now();
    loop {
        let (mut route_done, mut open_done) = (false, false);
        for (op, done) in [(route, &mut route_done), (open, &mut open_done)] {
            let row = operations::get(&state.db, op)
                .await
                .map_err(internal)?
                .ok_or_else(|| internal(std::io::Error::other("an operation vanished")))?;
            match row.status {
                operations::Status::Succeeded => *done = true,
                operations::Status::Failed => {
                    return Err(AppError::unavailable(
                        "webmail_failed",
                        "The node could not start webmail.",
                    ));
                }
                _ => {}
            }
        }
        if route_done && open_done {
            break;
        }
        if started.elapsed() > WEBMAIL_WAIT {
            return Err(AppError::unavailable(
                "node_busy",
                "Webmail is not ready yet; try again in a moment.",
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let ends = operations::get(&state.db, open)
        .await
        .map_err(internal)?
        .and_then(|r| r.output)
        .and_then(|o| o.get("link_expires_at").and_then(serde_json::Value::as_i64))
        .unwrap_or(0);
    Ok(Json(WebmailLink {
        url: format!("https://{host}/?_dhsignon={token}"),
        expires_at: rfc3339(ends),
    }))
}

// ----------------------------------------------------------------------------- forwarders

/// A forwarder.
#[derive(Debug, Serialize, ToSchema)]
pub struct ForwarderView {
    /// Id.
    pub id: Uuid,
    /// The address that sends on (`*@domain` is the catch-all).
    pub source: String,
    /// Where it goes.
    pub destination: String,
    /// The domain.
    pub domain: String,
    /// When it was made.
    pub created_at: String,
}

/// The forwarders of the account.
#[utoipa::path(
    get,
    path = "/api/v1/email/forwarders",
    tag = "email",
    operation_id = "clientListForwarders",
    security(("session" = [])),
    params(DomainQuery),
    responses((status = 200, description = "The forwarders", body = Vec<ForwarderView>))
)]
pub async fn list_forwarders(
    State(state): State<MailState>,
    who: Principal,
    Query(q): Query<DomainQuery>,
) -> Result<Json<Vec<ForwarderView>>, AppError> {
    let h = hosting(&state, &who).await?;
    let domain = q.domain.as_deref().map(str::to_ascii_lowercase);
    let items = store::forwarders_of_tenant(&state.db, h.tenant.id, domain.as_deref())
        .await
        .map_err(internal)?
        .into_iter()
        .map(|f| ForwarderView {
            id: f.id,
            source: format!("{}@{}", f.local_part, f.fqdn),
            destination: f.destination,
            domain: f.fqdn,
            created_at: created(f.created_at),
        })
        .collect();
    Ok(Json(items))
}

/// A forwarder to make.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ForwarderRequest {
    /// The address that sends on, `sales@example.com` (`*@example.com` for the catch-all).
    pub source: String,
    /// Where it goes: a full address.
    pub destination: String,
    /// The domain.
    pub domain: String,
}

/// Make a forwarder.
#[utoipa::path(
    post,
    path = "/api/v1/email/forwarders",
    tag = "email",
    operation_id = "clientCreateForwarder",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ForwarderRequest,
    responses(
        (status = 201, description = "Made", body = ForwarderView),
        (status = 409, description = "`address_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "An address is not valid", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_forwarder(
    State(state): State<MailState>,
    who: Principal,
    Json(req): Json<ForwarderRequest>,
) -> Result<(StatusCode, Json<ForwarderView>), AppError> {
    let h = hosting(&state, &who).await?;
    let domain = req.domain.trim().to_ascii_lowercase();
    let source = req.source.trim().to_ascii_lowercase();
    let local = match source.split_once('@') {
        Some((local, host)) if host == domain => local.to_owned(),
        None => source.clone(),
        _ => {
            return Err(field(
                "source",
                "wrong_domain",
                "The address must be in the domain.",
            ));
        }
    };
    if local != "*" && !local_ok(&local) {
        return Err(field("source", "invalid", "This is not a valid address."));
    }
    let destination = req.destination.trim().to_ascii_lowercase();
    if !external_address_ok(&destination) {
        return Err(field(
            "destination",
            "invalid",
            "This is not a valid address.",
        ));
    }
    if destination == format!("{local}@{domain}") {
        return Err(field(
            "destination",
            "loop",
            "A forwarder cannot send to itself.",
        ));
    }
    let md = mail_domain_for(&state, &h, &domain).await?;
    if md.state == "removing" {
        return Err(AppError::conflict(
            "domain_busy",
            "Mail of this domain is being removed.",
        ));
    }
    if local != "*"
        && store::mailbox_exists(&state.db, md.id, &local)
            .await
            .map_err(internal)?
    {
        return Err(AppError::conflict(
            "address_taken",
            "A mailbox has this address.",
        ));
    }
    let id = store::add_forwarder(&state.db, md.id, &local, &destination)
        .await
        .map_err(|e| match e {
            MakeError::Taken => {
                AppError::conflict("address_taken", "This forwarder exists already.")
            }
            MakeError::Database(e) => internal(e),
        })?;
    // The catch-all is part of the domain's settings, the rest is the list of forwarders.
    store::bump_forwarders(&state.db, md.id)
        .await
        .map_err(internal)?;
    if local == "*" {
        store::bump_domain(&state.db, md.id)
            .await
            .map_err(internal)?;
    }
    audit(
        &state,
        "forwarder.created",
        &who,
        Some(h.tenant.id),
        "forwarder",
        id,
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(ForwarderView {
            id,
            source: format!("{local}@{domain}"),
            destination,
            domain,
            created_at: created(now()),
        }),
    ))
}

/// Remove a forwarder.
#[utoipa::path(
    delete,
    path = "/api/v1/email/forwarders/{id}",
    tag = "email",
    operation_id = "clientDeleteForwarder",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Forwarder id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 204, description = "Removed"),
        (status = 404, description = "No such forwarder", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_forwarder(
    State(state): State<MailState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = hosting(&state, &who).await?;
    let f = store::forwarder(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("forwarder"))?;
    store::delete_forwarder(&state.db, id)
        .await
        .map_err(internal)?;
    store::bump_forwarders(&state.db, f.mail_domain_id)
        .await
        .map_err(internal)?;
    if f.local_part == "*" {
        store::bump_domain(&state.db, f.mail_domain_id)
            .await
            .map_err(internal)?;
    }
    audit(
        &state,
        "forwarder.deleted",
        &who,
        Some(h.tenant.id),
        "forwarder",
        id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------------------- autoresponders

/// An automatic reply.
#[derive(Debug, Serialize, ToSchema)]
pub struct AutoresponderView {
    /// Id (the mailbox's).
    pub id: Uuid,
    /// The mailbox.
    pub email: String,
    /// The domain.
    pub domain: String,
    /// The subject of the reply.
    pub subject: String,
    /// The text of the reply.
    pub body: String,
    /// Whether it replies now.
    pub is_active: bool,
    /// From when.
    pub start_date: Option<String>,
    /// Until when.
    pub end_date: Option<String>,
}

fn autoresponder_view(m: Mailbox) -> Option<AutoresponderView> {
    let (subject, body) = (m.ar_subject?, m.ar_body?);
    let t = now();
    let active = m.ar_from.is_none_or(|f| f <= t) && m.ar_until.is_none_or(|u| u > t);
    Some(AutoresponderView {
        id: m.id,
        email: format!("{}@{}", m.local_part, m.fqdn),
        domain: m.fqdn,
        subject,
        body,
        is_active: active,
        start_date: m.ar_from.map(rfc3339),
        end_date: m.ar_until.map(rfc3339),
    })
}

/// The automatic replies of the account.
#[utoipa::path(
    get,
    path = "/api/v1/email/autoresponders",
    tag = "email",
    operation_id = "clientListAutoresponders",
    security(("session" = [])),
    params(DomainQuery),
    responses((status = 200, description = "The replies", body = Vec<AutoresponderView>))
)]
pub async fn list_autoresponders(
    State(state): State<MailState>,
    who: Principal,
    Query(q): Query<DomainQuery>,
) -> Result<Json<Vec<AutoresponderView>>, AppError> {
    let h = hosting(&state, &who).await?;
    let domain = q.domain.as_deref().map(str::to_ascii_lowercase);
    let items = store::mailboxes_of_tenant(&state.db, h.tenant.id, domain.as_deref())
        .await
        .map_err(internal)?
        .into_iter()
        .filter_map(autoresponder_view)
        .collect();
    Ok(Json(items))
}

/// An automatic reply to set.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AutoresponderRequest {
    /// The mailbox.
    pub email: String,
    /// The domain.
    pub domain: String,
    /// The subject of the reply (up to 200 characters).
    pub subject: String,
    /// The text of the reply (up to 5000 characters).
    pub body: String,
    /// From when (`2026-12-24` or a full time); at once when left out.
    #[serde(default)]
    pub start_date: Option<String>,
    /// Until when; no end when left out.
    #[serde(default)]
    pub end_date: Option<String>,
}

/// A date or time as the page writes it, in Unix seconds (UTC).
fn seconds_of(text: &str) -> Option<i64> {
    let text = text.trim();
    let (date, time) = match text.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (text, None),
    };
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(1970..=9999).contains(&year)
    {
        return None;
    }
    let (mut hour, mut minute, mut second) = (0, 0, 0);
    if let Some(time) = time {
        let time = time.trim_end_matches('Z');
        let time = time.split(['.', '+']).next()?;
        let mut t = time.split(':');
        hour = t.next()?.parse::<i64>().ok()?;
        minute = t.next().map_or(Some(0), |m| m.parse::<i64>().ok())?;
        second = t.next().map_or(Some(0), |s| s.parse::<i64>().ok())?;
        if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..60).contains(&second) {
            return None;
        }
    }
    // Days since 1970-01-01 (the civil-from-days algorithm, inverted).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Set a mailbox's automatic reply.
#[utoipa::path(
    post,
    path = "/api/v1/email/autoresponders",
    tag = "email",
    operation_id = "clientSetAutoresponder",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = AutoresponderRequest,
    responses(
        (status = 201, description = "Set", body = AutoresponderView),
        (status = 404, description = "No such mailbox", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A field is not valid", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_autoresponder(
    State(state): State<MailState>,
    who: Principal,
    Json(req): Json<AutoresponderRequest>,
) -> Result<(StatusCode, Json<AutoresponderView>), AppError> {
    let h = hosting(&state, &who).await?;
    let subject = req.subject.trim();
    let body = req.body.trim();
    if subject.is_empty() || subject.chars().count() > 200 || subject.contains(['\r', '\n']) {
        return Err(field(
            "subject",
            "invalid",
            "The subject is 1 to 200 characters on one line.",
        ));
    }
    if body.is_empty() || body.chars().count() > 5000 {
        return Err(field(
            "body",
            "invalid",
            "The text is 1 to 5000 characters.",
        ));
    }
    let parse = |name: &'static str, text: &Option<String>| match text.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(t) => seconds_of(t)
            .map(Some)
            .ok_or_else(|| field(name, "invalid", "This is not a date.")),
    };
    let from = parse("start_date", &req.start_date)?;
    let until = parse("end_date", &req.end_date)?;
    if let (Some(f), Some(u)) = (from, until)
        && u <= f
    {
        return Err(field(
            "end_date",
            "before_start",
            "The end must be after the start.",
        ));
    }
    let domain = req.domain.trim().to_ascii_lowercase();
    let local = req
        .email
        .trim()
        .to_ascii_lowercase()
        .split_once('@')
        .map_or_else(
            || req.email.trim().to_ascii_lowercase(),
            |(l, _)| l.to_owned(),
        );
    let md = existing_mail_domain(&state, &h, &domain).await?;
    let mailbox = store::mailboxes_of_domain(&state.db, md.id)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|m| m.local_part == local && m.state != "removing")
        .ok_or_else(|| AppError::not_found("mailbox"))?;
    store::set_autoresponder(&state.db, mailbox.id, Some((subject, body, from, until)))
        .await
        .map_err(internal)?;
    audit(
        &state,
        "autoresponder.set",
        &who,
        Some(h.tenant.id),
        "mailbox",
        mailbox.id,
    )
    .await?;
    let made = store::mailbox(&state.db, h.tenant.id, mailbox.id)
        .await
        .map_err(internal)?
        .and_then(autoresponder_view)
        .ok_or_else(|| AppError::not_found("mailbox"))?;
    Ok((StatusCode::CREATED, Json(made)))
}

/// Turn a mailbox's automatic reply off.
#[utoipa::path(
    delete,
    path = "/api/v1/email/autoresponders/{id}",
    tag = "email",
    operation_id = "clientDeleteAutoresponder",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "The mailbox's id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 204, description = "Off"),
        (status = 404, description = "No such reply", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_autoresponder(
    State(state): State<MailState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = hosting(&state, &who).await?;
    store::mailbox(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .filter(|m| m.ar_subject.is_some())
        .ok_or_else(|| AppError::not_found("autoresponder"))?;
    store::set_autoresponder(&state.db, id, None)
        .await
        .map_err(internal)?;
    audit(
        &state,
        "autoresponder.removed",
        &who,
        Some(h.tenant.id),
        "mailbox",
        id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------------------- settings

/// What a mail program needs to know.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClientConfig {
    /// The server to read mail from.
    pub incoming_server: String,
    /// IMAP over TLS.
    pub incoming_imap_port: u16,
    /// POP3 (0: the server does not offer it).
    pub incoming_pop3_port: u16,
    /// How the connection is secured.
    pub incoming_security: String,
    /// The server to send mail through.
    pub outgoing_server: String,
    /// SMTP over TLS (submission).
    pub outgoing_smtp_port: u16,
    /// Submission with STARTTLS (0: the server does not offer it).
    pub outgoing_submission_port: u16,
    /// How the connection is secured.
    pub outgoing_security: String,
    /// Always true: the full address and the password.
    pub auth_required: bool,
}

/// The name the account's mail server has.
async fn server_name(state: &MailState, node_id: Uuid) -> Result<String, AppError> {
    if let Some(server) = store::server(&state.db, node_id).await.map_err(internal)? {
        return Ok(server.hostname);
    }
    Ok(rules(&state.settings, &state.db).await?.hostname)
}

/// What to set a mail program to.
#[utoipa::path(
    get,
    path = "/api/v1/email/config",
    tag = "email",
    operation_id = "clientMailConfig",
    security(("session" = [])),
    params(DomainQuery),
    responses((status = 200, description = "The settings", body = ClientConfig))
)]
pub async fn client_config(
    State(state): State<MailState>,
    who: Principal,
    Query(q): Query<DomainQuery>,
) -> Result<Json<ClientConfig>, AppError> {
    let h = hosting(&state, &who).await?;
    let mut host = server_name(&state, h.tenant.node_id).await?;
    if host.is_empty() {
        host = format!("mail.{}", q.domain.unwrap_or_default().to_ascii_lowercase());
    }
    Ok(Json(ClientConfig {
        incoming_server: host.clone(),
        incoming_imap_port: 993,
        incoming_pop3_port: 0,
        incoming_security: "SSL/TLS".to_owned(),
        outgoing_server: host,
        outgoing_smtp_port: 465,
        outgoing_submission_port: 0,
        outgoing_security: "SSL/TLS".to_owned(),
        auth_required: true,
    }))
}

// ----------------------------------------------------------------------------------- admin

/// A message in the outgoing queue.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueueItemView {
    /// The queue's id for it.
    pub id: String,
    /// The sender.
    pub sender: String,
    /// The recipients, separated by commas.
    pub recipient: String,
    /// `queued` or `deferred`.
    pub status: String,
    /// Why the last try failed.
    pub error_message: Option<String>,
    /// When it was queued.
    pub queued_at: String,
    /// How many times it was tried.
    pub retries: u32,
    /// Its size in bytes.
    pub size_bytes: u64,
}

async fn reported_queue(state: &MailState) -> Result<Vec<(Uuid, MailQueueItem)>, AppError> {
    let mut out = Vec::new();
    for (node, items, _) in store::queues(&state.db).await.map_err(internal)? {
        let items: Vec<MailQueueItem> = serde_json::from_value(items).unwrap_or_default();
        out.extend(items.into_iter().map(|i| (node, i)));
    }
    Ok(out)
}

/// The outgoing mail queue of every node, as it was last reported.
#[utoipa::path(
    get,
    path = "/api/v1/admin/email/queue",
    tag = "email",
    operation_id = "adminMailQueue",
    security(("session" = [])),
    responses((status = 200, description = "The queue", body = Vec<QueueItemView>))
)]
pub async fn queue(
    State(state): State<MailState>,
    _who: Require<EmailRead>,
) -> Result<Json<Vec<QueueItemView>>, AppError> {
    let items = reported_queue(&state)
        .await?
        .into_iter()
        .map(|(_, i)| QueueItemView {
            id: i.id,
            sender: i.sender,
            recipient: i.recipients.join(", "),
            status: i.status,
            error_message: i.error,
            queued_at: rfc3339(i.queued_at),
            retries: i.retries,
            size_bytes: i.size_bytes,
        })
        .collect();
    Ok(Json(items))
}

async fn queue_action(
    state: &MailState,
    node_id: Uuid,
    action: &str,
    ids: Vec<String>,
) -> Result<(), AppError> {
    let op = Operation::MailQueueAction {
        action: action.to_owned(),
        ids,
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let key = format!("mailqueue:{node_id}:{action}:{}", Uuid::new_v4());
    operations::enqueue(
        &state.db,
        &NewOperation {
            node_id,
            kind: op.type_name(),
            tenant_id: None,
            payload: &payload,
            idempotency_key: &key,
            created_by: None,
            ttl_secs: 3600,
            max_attempts: 3,
        },
    )
    .await
    .map_err(internal)?;
    Ok(())
}

/// What a queue action did.
#[derive(Debug, Serialize, ToSchema)]
pub struct QueueChanged {
    /// Always true.
    pub success: bool,
    /// How many messages it was asked about.
    pub count: usize,
}

/// Retry every queued message now.
#[utoipa::path(
    post,
    path = "/api/v1/admin/email/queue/flush",
    tag = "email",
    operation_id = "adminFlushMailQueue",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 200, description = "Asked", body = QueueChanged))
)]
pub async fn flush_queue(
    State(state): State<MailState>,
    who: Require<EmailManage>,
) -> Result<Json<QueueChanged>, AppError> {
    let mut count = 0;
    for (node, items, _) in store::queues(&state.db).await.map_err(internal)? {
        count += items.as_array().map_or(0, Vec::len);
        queue_action(&state, node, "flush", Vec::new()).await?;
    }
    audit(
        &state,
        "mail_queue.flushed",
        &who.0,
        None,
        "mail_queue",
        Uuid::nil(),
    )
    .await?;
    Ok(Json(QueueChanged {
        success: true,
        count,
    }))
}

/// Remove one queued message.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/email/queue/{id}",
    tag = "email",
    operation_id = "adminDeleteQueuedMail",
    security(("session" = [])),
    params(("id" = String, Path, description = "The queue's id for the message"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 204, description = "Asked"),
        (status = 404, description = "No such message", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_queued(
    State(state): State<MailState>,
    who: Require<EmailManage>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let node = reported_queue(&state)
        .await?
        .into_iter()
        .find(|(_, i)| i.id == id)
        .map(|(n, _)| n)
        .ok_or_else(|| AppError::not_found("queued_message"))?;
    queue_action(&state, node, "delete", vec![id]).await?;
    audit(
        &state,
        "mail_queue.deleted",
        &who.0,
        None,
        "mail_queue",
        Uuid::nil(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// How one domain's mail stands.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeliverabilityView {
    /// The domain.
    pub domain: String,
    /// `pass`, `neutral`, `fail` or `missing`.
    pub spf_status: String,
    /// `pass`, `fail` or `missing`.
    pub dkim_status: String,
    /// `pass`, `fail` or `missing`.
    pub dmarc_status: String,
    /// `pass` or `fail`.
    pub mx_status: String,
    /// Whether the mail server's address is on a blacklist (not checked yet: always false).
    pub rbl_blacklisted: bool,
    /// The blacklists it is on.
    pub rbl_listings: Vec<String>,
    /// What to do about what is wrong.
    pub recommendations: Vec<String>,
}

fn trimmed(text: &str) -> String {
    text.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Compares what the mail server asked for with what the domain's zone holds.
fn check_zone(
    md: &MailDomain,
    zone: &[builtpanel_database::dns::StoredRecord],
) -> DeliverabilityView {
    use builtpanel_domain::DnsRecordType as T;
    let wanted: Vec<builtpanel_node_protocol::MailDnsRecord> =
        serde_json::from_value(md.records.clone()).unwrap_or_default();
    let mut notes = Vec::new();
    let in_zone = |name: &str, rtype: T| -> Vec<&builtpanel_domain::DnsRecord> {
        zone.iter()
            .map(|r| &r.record)
            .filter(|r| r.name == name && r.rtype == rtype)
            .collect()
    };
    let wanted_of = |name_is: &dyn Fn(&str) -> bool,
                     rtype: &str|
     -> Vec<&builtpanel_node_protocol::MailDnsRecord> {
        wanted
            .iter()
            .filter(|w| w.rtype == rtype && name_is(&w.name))
            .collect()
    };

    let mx_status = {
        let want = wanted_of(&|n| n == "@", "MX");
        let have = in_zone("@", T::Mx);
        if !want.is_empty()
            && want.iter().all(|w| {
                have.iter()
                    .any(|h| trimmed(&h.content) == trimmed(&w.content))
            })
        {
            "pass"
        } else {
            notes.push("Point the domain's MX record at the mail server.".to_owned());
            "fail"
        }
    };
    let spf_status = {
        let have: Vec<_> = in_zone("@", T::Txt)
            .into_iter()
            .filter(|r| r.content.starts_with("v=spf1"))
            .collect();
        let want = wanted_of(&|n| n == "@", "TXT")
            .into_iter()
            .find(|w| w.content.starts_with("v=spf1"));
        match (have.first(), want) {
            (None, _) => {
                notes.push("Add the SPF record to the domain's DNS.".to_owned());
                "missing"
            }
            (Some(h), Some(w)) if h.content == w.content => "pass",
            _ => {
                notes.push(
                    "The domain's SPF record is not the one the mail server asks for.".to_owned(),
                );
                "neutral"
            }
        }
    };
    let dkim_status = {
        let want = wanted_of(&|n| n.ends_with("._domainkey"), "TXT");
        if want.is_empty() {
            "missing"
        } else {
            let mut status = "pass";
            for w in want {
                let have = in_zone(&w.name, T::Txt);
                if have.is_empty() {
                    status = "missing";
                    notes.push(
                        "Add the DKIM record to the domain's DNS so outgoing mail is signed."
                            .to_owned(),
                    );
                    break;
                }
                if !have.iter().any(|h| h.content == w.content) {
                    status = "fail";
                    notes.push(
                        "The DKIM record in DNS is not the mail server's current key.".to_owned(),
                    );
                }
            }
            status
        }
    };
    let dmarc_status = if in_zone("_dmarc", T::Txt)
        .iter()
        .any(|r| r.content.starts_with("v=DMARC1"))
    {
        "pass"
    } else {
        notes.push("Add a DMARC record for _dmarc.".to_owned());
        "missing"
    };
    if md.held {
        notes.push("Outgoing mail of this domain is held.".to_owned());
    }
    notes.push("Blacklists are not checked from the panel yet.".to_owned());
    DeliverabilityView {
        domain: md.fqdn.clone(),
        spf_status: spf_status.to_owned(),
        dkim_status: dkim_status.to_owned(),
        dmarc_status: dmarc_status.to_owned(),
        mx_status: mx_status.to_owned(),
        rbl_blacklisted: false,
        rbl_listings: Vec::new(),
        recommendations: notes,
    }
}

/// How the mail domains stand: their MX, SPF, DKIM and DMARC records against what the mail
/// server asks for.
#[utoipa::path(
    get,
    path = "/api/v1/admin/email/deliverability",
    tag = "email",
    operation_id = "adminMailDeliverability",
    security(("session" = [])),
    responses((status = 200, description = "One entry per mail domain", body = Vec<DeliverabilityView>))
)]
pub async fn deliverability(
    State(state): State<MailState>,
    _who: Require<EmailRead>,
) -> Result<Json<Vec<DeliverabilityView>>, AppError> {
    let mut out = Vec::new();
    for md in store::all_domains(&state.db).await.map_err(internal)? {
        if md.state == "removing" {
            continue;
        }
        let zone = match match md.domain_id {
            Some(id) => builtpanel_database::dns::zone_of_domain(&state.db, id)
                .await
                .map_err(internal)?,
            None => None,
        } {
            Some(z) => builtpanel_database::dns::records(&state.db, z.id)
                .await
                .map_err(internal)?,
            None => Vec::new(),
        };
        out.push(check_zone(&md, &zone));
    }
    Ok(Json(out))
}

/// A mail domain.
#[derive(Debug, Serialize, ToSchema)]
pub struct MailDomainView {
    /// Id.
    pub id: Uuid,
    /// The domain.
    pub domain: String,
    /// The hosting account (none once it is deleted).
    pub tenant_id: Option<Uuid>,
    /// `pending`, `active`, `error` or `removing`.
    pub state: String,
    /// Why the mail server refused the last change.
    pub error: Option<String>,
    /// Most messages an hour.
    pub per_hour: Option<i32>,
    /// Whether outgoing mail is held.
    pub held: bool,
    /// Why it is held.
    pub held_reason: Option<String>,
    /// Whether the DNS records are in the zone.
    pub dns_applied: bool,
}

fn domain_view(d: MailDomain) -> MailDomainView {
    MailDomainView {
        id: d.id,
        domain: d.fqdn,
        tenant_id: d.tenant_id,
        state: d.state,
        error: d.last_error,
        per_hour: d.per_hour,
        held: d.held,
        held_reason: d.held_reason,
        dns_applied: d.dns_applied,
    }
}

/// Every mail domain.
#[utoipa::path(
    get,
    path = "/api/v1/admin/email/domains",
    tag = "email",
    operation_id = "adminListMailDomains",
    security(("session" = [])),
    responses((status = 200, description = "The mail domains", body = Vec<MailDomainView>))
)]
pub async fn list_domains(
    State(state): State<MailState>,
    _who: Require<EmailRead>,
) -> Result<Json<Vec<MailDomainView>>, AppError> {
    Ok(Json(
        store::all_domains(&state.db)
            .await
            .map_err(internal)?
            .into_iter()
            .map(domain_view)
            .collect(),
    ))
}

/// Why a domain is held.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HoldRequest {
    /// The reason, shown to the account (up to 300 characters).
    pub reason: String,
}

/// Hold a domain's outgoing mail (a limit of one message a day).
#[utoipa::path(
    post,
    path = "/api/v1/admin/email/domains/{id}/hold",
    tag = "email",
    operation_id = "adminHoldMailDomain",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Mail domain id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = HoldRequest,
    responses(
        (status = 200, description = "Held", body = MailDomainView),
        (status = 404, description = "No such mail domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn hold_domain(
    State(state): State<MailState>,
    who: Require<EmailManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<HoldRequest>,
) -> Result<Json<MailDomainView>, AppError> {
    let reason = req.reason.trim();
    if reason.is_empty() || reason.chars().count() > 300 {
        return Err(field(
            "reason",
            "invalid",
            "Give a reason of up to 300 characters.",
        ));
    }
    store::domain(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("mail_domain"))?;
    store::set_held(&state.db, id, true, Some(reason))
        .await
        .map_err(internal)?;
    audit(&state, "mail_domain.held", &who.0, None, "mail_domain", id).await?;
    let d = store::domain(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("mail_domain"))?;
    Ok(Json(domain_view(d)))
}

/// Let a held domain send again.
#[utoipa::path(
    post,
    path = "/api/v1/admin/email/domains/{id}/release",
    tag = "email",
    operation_id = "adminReleaseMailDomain",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Mail domain id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 200, description = "Released", body = MailDomainView),
        (status = 404, description = "No such mail domain", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn release_domain(
    State(state): State<MailState>,
    who: Require<EmailManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<MailDomainView>, AppError> {
    store::domain(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("mail_domain"))?;
    store::set_held(&state.db, id, false, None)
        .await
        .map_err(internal)?;
    audit(
        &state,
        "mail_domain.released",
        &who.0,
        None,
        "mail_domain",
        id,
    )
    .await?;
    let d = store::domain(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("mail_domain"))?;
    Ok(Json(domain_view(d)))
}

/// A node's mail server.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerView {
    /// The node.
    pub node_id: Uuid,
    /// The name it should say it is.
    pub hostname: String,
    /// The name it has been told.
    pub applied_hostname: Option<String>,
    /// Why the node refused.
    pub error: Option<String>,
}

/// The nodes' mail servers.
#[utoipa::path(
    get,
    path = "/api/v1/admin/email/servers",
    tag = "email",
    operation_id = "adminListMailServers",
    security(("session" = [])),
    responses((status = 200, description = "The servers", body = Vec<ServerView>))
)]
pub async fn list_servers(
    State(state): State<MailState>,
    _who: Require<EmailRead>,
) -> Result<Json<Vec<ServerView>>, AppError> {
    Ok(Json(
        store::servers(&state.db)
            .await
            .map_err(internal)?
            .into_iter()
            .map(|s| ServerView {
                node_id: s.node_id,
                hostname: s.hostname,
                applied_hostname: s.applied_hostname,
                error: s.last_error,
            })
            .collect(),
    ))
}

/// A name for a node's mail server.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ServerRequest {
    /// The name (it must point at the node).
    #[schema(example = "mail.example.com")]
    pub hostname: String,
}

/// Name a node's mail server.
#[utoipa::path(
    put,
    path = "/api/v1/admin/email/servers/{node_id}",
    tag = "email",
    operation_id = "adminSetMailServer",
    security(("session" = [])),
    params(("node_id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ServerRequest,
    responses(
        (status = 200, description = "Set", body = ServerView),
        (status = 422, description = "Not a name", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn set_server(
    State(state): State<MailState>,
    who: Require<EmailManage>,
    Path(node_id): Path<Uuid>,
    Json(req): Json<ServerRequest>,
) -> Result<Json<ServerView>, AppError> {
    let host = Fqdn::parse(&req.hostname)
        .map_err(|_| field("hostname", "invalid", "This is not a host name."))?;
    store::want_hostname(&state.db, node_id, host.as_str())
        .await
        .map_err(
            |e| match e.as_database_error().and_then(|d| d.code()).as_deref() {
                Some("23503") => AppError::not_found("node"),
                _ => internal(e),
            },
        )?;
    audit(&state, "mail_server.named", &who.0, None, "node", node_id).await?;
    let s = store::server(&state.db, node_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("node"))?;
    Ok(Json(ServerView {
        node_id: s.node_id,
        hostname: s.hostname,
        applied_hostname: s.applied_hostname,
        error: s.last_error,
    }))
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(
        list_mailboxes, create_mailbox, delete_mailbox, change_password, open_webmail,
        list_forwarders, create_forwarder, delete_forwarder,
        list_autoresponders, create_autoresponder, delete_autoresponder, client_config,
        queue, flush_queue, delete_queued, deliverability,
        list_domains, hold_domain, release_domain, list_servers, set_server
    ),
    components(schemas(
        MailboxView, MailboxRequest, PasswordRequest, Done, WebmailLink, ForwarderView, ForwarderRequest,
        AutoresponderView, AutoresponderRequest, ClientConfig, QueueItemView, QueueChanged,
        DeliverabilityView, MailDomainView, HoldRequest, ServerView, ServerRequest
    )),
    tags((name = "email", description = "Email"))
)]
pub struct MailApi;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_come_out_as_unix_seconds() {
        assert_eq!(seconds_of("1970-01-01"), Some(0));
        assert_eq!(seconds_of("2026-11-18"), Some(1_794_960_000));
        assert_eq!(
            seconds_of("2026-11-18T01:02:03Z"),
            Some(1_794_960_000 + 3723)
        );
        assert_eq!(
            seconds_of("2026-11-18T01:02:03.500Z"),
            Some(1_794_960_000 + 3723)
        );
        assert_eq!(seconds_of("2026-13-01"), None);
        assert_eq!(seconds_of("tomorrow"), None);
    }

    #[test]
    fn local_parts_and_outside_addresses_are_checked() {
        for ok in ["info", "a.b", "sales-eu", "x-1"] {
            assert!(local_ok(ok), "{ok}");
        }
        for bad in ["", ".a", "a.", "A", "a b", "a@b", "a+b", &"x".repeat(65)] {
            assert!(!local_ok(bad), "{bad}");
        }
        assert!(external_address_ok("team@crm.example.com"));
        assert!(!external_address_ok("nobody"));
        assert!(!external_address_ok("a b@example.com"));
        assert!(!external_address_ok("a@localhost"));
    }
}
