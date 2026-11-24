//! Client accounts (T04.12): the people who host sites, and the tenant each owns.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET    | `/api/v1/admin/users` | `admin.users.read` |
//! | POST   | `/api/v1/admin/users` | `admin.users.manage`, sudo mode |
//! | GET    | `/api/v1/admin/users/{id}` | `admin.users.read` |
//! | PATCH  | `/api/v1/admin/users/{id}` | `admin.users.manage`, sudo mode |
//! | DELETE | `/api/v1/admin/users/{id}` | `admin.users.manage`, sudo mode |
//! | POST   | `/api/v1/admin/users/{id}/plan` | `admin.users.manage`, sudo mode |
//! | POST   | `/api/v1/admin/users/{id}/suspend` | `admin.users.manage`, sudo mode |
//! | POST   | `/api/v1/admin/users/{id}/unsuspend` | `admin.users.manage`, sudo mode |
//! | POST   | `/api/v1/admin/users/{id}/password` | `admin.users.manage`, sudo mode |
//! | POST   | `/api/v1/admin/users/{id}/retry-setup` | `admin.users.manage`, sudo mode |
//!
//! Creating a user writes the account, its tenant (with a numeric id taken on the
//! chosen node) and the `set_up_tenant` operation in **one transaction**, so there
//! is never an account without the work that sets it up, nor work without an
//! account. The answer carries the `operation_id`; when the node reports
//! success the operations ledger itself moves the tenant to `active`
//! (migration 17), and to `failed` when the operation fails for good.
//!
//! Suspend, unsuspend, change of plan and delete work the same way: the request
//! changes the account and queues the node's work; the tenant's status follows
//! the ledger. Delete is soft: the stack goes at once, the data is kept for
//! `purge_after_days` and then purged by a second operation.

use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_auth_core::{password::PasswordHasher, policy::PasswordPolicy};
use builtpanel_database::{
    accounts::{self, InsertAccountError, NewAccount},
    operations::{self, EnqueueError, NewOperation},
    plans::{self, Plan},
    rbac::{self, AssignRole},
    skeleton,
    sqlx::{PgConnection, PgPool},
    tenants::{self, CreateError},
    users::{self, Filter, UpdateError, User},
};
use builtpanel_domain::{AccountKind, Email, TenantStatus, TenantUsername, Username};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::{Operation as NodeOperation, SkeletonFile, check_skeleton};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{UsersImpersonate, UsersManage, UsersRead},
};
use builtpanel_session::{RevokeReason, SessionStore, rfc3339};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

pub mod reseller;

/// The users module's manifest text.
pub const MANIFEST: &str = include_str!("../../../../modules/users/module.toml");

/// Entries per page when `limit` is not given.
pub const DEFAULT_LIMIT: i64 = 50;
/// Most entries per page.
pub const MAX_LIMIT: i64 = 100;
/// Name of the built-in role every client account gets.
const CLIENT_ROLE: &str = "client";
/// How long a queued piece of work stays worth sending.
const OPERATION_TTL_SECS: i32 = 6 * 3600;
/// Most deliveries of one piece of work.
const OPERATION_ATTEMPTS: i32 = 5;

/// Dependencies of the users handlers.
#[derive(Clone)]
pub struct UsersState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// Password hasher with the configured cost.
    pub hasher: PasswordHasher,
    /// Days a deleted user's data is kept before it is purged.
    pub purge_after_days: i32,
}

impl FromRef<UsersState> for PgPool {
    fn from_ref(s: &UsersState) -> Self {
        s.db.clone()
    }
}

impl FromRef<UsersState> for SessionStore {
    fn from_ref(s: &UsersState) -> Self {
        s.sessions.clone()
    }
}

/// Routes with full paths; merge into the application router.
pub fn router() -> Router<UsersState> {
    Router::new()
        .route("/api/v1/admin/users", get(list_users).post(create_user))
        .route(
            "/api/v1/admin/users/{id}",
            get(get_user).patch(update_user).delete(delete_user),
        )
        .route("/api/v1/admin/users/{id}/plan", post(change_plan))
        .route("/api/v1/admin/users/{id}/suspend", post(suspend_user))
        .route("/api/v1/admin/users/{id}/unsuspend", post(unsuspend_user))
        .route("/api/v1/admin/users/{id}/password", post(reset_password))
        .route("/api/v1/admin/users/{id}/retry-setup", post(retry_setup))
        .route("/api/v1/admin/users/{id}/impersonate", post(impersonate))
        .route(
            "/api/v1/admin/skeleton-files",
            get(list_skeleton_files)
                .put(put_skeleton_file)
                .delete(delete_skeleton_file),
        )
}

pub(crate) fn internal(
    err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
) -> AppError {
    AppError::internal(err)
}

// ------------------------------------------------------------------- views

/// A user's hosting account on a node.
#[derive(Debug, Serialize, ToSchema)]
pub struct TenantView {
    /// Tenant id.
    pub id: Uuid,
    /// System user name on the node.
    #[schema(example = "alice")]
    pub username: String,
    /// `provisioning`, `active`, `suspended`, `deleting` or `failed`.
    #[schema(example = "active")]
    pub status: String,
    /// Plan id.
    pub plan_id: Uuid,
    /// Plan name.
    pub plan_name: String,
    /// Node id.
    pub node_id: Uuid,
    /// Node name.
    pub node_name: String,
    /// When the data will be purged (RFC 3339, UTC); only for deleted users.
    #[schema(format = DateTime)]
    pub purge_after: Option<String>,
    /// What the account uses of its plan; only in the list of users.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageShares>,
}

/// How much of its plan a hosting account uses, in whole percent (above 100 when it is over). A
/// field is absent when the plan has no limit for it or nothing was measured lately.
#[derive(Debug, Serialize, ToSchema)]
pub struct UsageShares {
    /// Memory (the newest reading, at most 15 minutes old).
    pub memory_percent: Option<i32>,
    /// CPU (the newest reading, at most 15 minutes old).
    pub cpu_percent: Option<i32>,
    /// Disk space (as the node last measured it).
    pub disk_percent: Option<i32>,
    /// Files and folders (as the node last measured them).
    pub inodes_percent: Option<i32>,
}

/// A client account.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserView {
    /// Id.
    pub id: Uuid,
    /// Login name.
    #[schema(example = "alice")]
    pub username: String,
    /// Email address.
    pub email: String,
    /// Interface language (`en` or `bn`).
    pub locale: String,
    /// `active` (may log in), `suspended` or `disabled`.
    pub account_status: String,
    /// The hosting account, if it has one.
    pub tenant: Option<TenantView>,
    /// Creation time (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
}

fn view(user: User) -> UserView {
    UserView {
        id: user.id,
        username: user.username,
        email: user.email,
        locale: user.locale,
        account_status: user.account_status,
        tenant: user.tenant.map(|t| TenantView {
            id: t.id,
            username: t.username,
            status: t.status.as_str().to_owned(),
            plan_id: t.plan_id,
            plan_name: t.plan_name,
            node_id: t.node_id,
            node_name: t.node_name,
            purge_after: t.purge_after.map(rfc3339),
            usage: None,
        }),
        created_at: rfc3339(user.created_at),
    }
}

/// What a change answers: the user as it is now and, when the node has work to
/// do, the operation to follow.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserChange {
    /// The user.
    pub user: UserView,
    /// The queued work; follow it in the operations ledger. `None` when nothing
    /// had to be sent to a node.
    pub operation_id: Option<Uuid>,
}

/// A page of users.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserList {
    /// The users, newest first.
    pub items: Vec<UserView>,
    /// How many users match in all.
    pub total: i64,
}

// ------------------------------------------------------------------- helpers

fn entry(action: &'static str, who: &Principal, target: Uuid) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target("user", target)
}

async fn load(state: &UsersState, id: Uuid) -> Result<User, AppError> {
    users::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))
}

fn tenant_of(user: &User) -> Result<&users::UserTenant, AppError> {
    user.tenant.as_ref().ok_or_else(|| {
        AppError::conflict("no_tenant", "This user has no hosting account to change.")
    })
}

/// The numbers a plan gives a stack, in the types the node protocol uses.
struct Wire {
    cpu_millis: Option<u32>,
    memory_bytes: Option<u64>,
    pids: Option<u32>,
    disk_bytes: Option<u64>,
    inodes: Option<u64>,
}

fn wire(plan: &Plan) -> Result<Wire, AppError> {
    let bad = || internal(std::io::Error::other("a plan holds a negative resource"));
    Ok(Wire {
        cpu_millis: plan
            .cpu_millis
            .map(u32::try_from)
            .transpose()
            .map_err(|_| bad())?,
        memory_bytes: plan
            .memory_bytes
            .map(u64::try_from)
            .transpose()
            .map_err(|_| bad())?,
        pids: plan
            .pids
            .map(u32::try_from)
            .transpose()
            .map_err(|_| bad())?,
        disk_bytes: plan
            .disk_bytes
            .map(u64::try_from)
            .transpose()
            .map_err(|_| bad())?,
        inodes: plan
            .inodes
            .map(u64::try_from)
            .transpose()
            .map_err(|_| bad())?,
    })
}

/// Queues work for a node inside the caller's transaction.
async fn queue(
    conn: &mut PgConnection,
    node_id: Uuid,
    tenant_id: Uuid,
    op: &NodeOperation,
    who: &Principal,
) -> Result<Uuid, AppError> {
    let payload = serde_json::to_value(op).map_err(internal)?;
    // Every request is its own piece of work: a fresh key. (Retrying the HTTP
    // request is the client's business; the node side is repeatable anyway.)
    let key = format!("{}:{tenant_id}:{}", op.type_name(), Uuid::now_v7());
    let queued = operations::enqueue_in(
        conn,
        &NewOperation {
            node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant_id),
            payload: &payload,
            idempotency_key: &key,
            created_by: Some(who.account_id()),
            ttl_secs: OPERATION_TTL_SECS,
            max_attempts: OPERATION_ATTEMPTS,
        },
    )
    .await
    .map_err(|e| match e {
        EnqueueError::UnknownNode => AppError::conflict(
            "node_unavailable",
            "The node of this user no longer exists.",
        ),
        other => internal(other),
    })?;
    Ok(queued.operation().id)
}

/// The default files, as the node wants them.
async fn skeleton_for_new_tenant(conn: &mut PgConnection) -> Result<Vec<SkeletonFile>, AppError> {
    Ok(skeleton::list_in(conn)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|f| SkeletonFile {
            path: f.path,
            content: f.content,
        })
        .collect())
}

fn set_up_operation(
    tenant_id: Uuid,
    username: &str,
    uid: i32,
    plan: &Plan,
    skeleton: Vec<SkeletonFile>,
) -> Result<NodeOperation, AppError> {
    let w = wire(plan)?;
    Ok(NodeOperation::SetUpTenant {
        tenant_id,
        username: username.to_owned(),
        uid: u32::try_from(uid).map_err(|_| internal(std::io::Error::other("negative uid")))?,
        disk_bytes: w.disk_bytes,
        inodes: w.inodes,
        cpu_millis: w.cpu_millis,
        memory_bytes: w.memory_bytes,
        pids: w.pids,
        skeleton,
    })
}

/// A system name from a login name: letters and digits only.
fn derived_tenant_name(username: &str) -> String {
    username
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

// ----------------------------------------------------------------------- list

/// Filters and paging of the list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// Entries per page, 1..=100 (default 50).
    pub limit: Option<String>,
    /// Entries to skip (default 0).
    pub offset: Option<String>,
    /// Hosting status: `provisioning`, `active`, `suspended`, `deleting`, `failed`.
    pub status: Option<String>,
    /// Only users on this plan.
    pub plan_id: Option<Uuid>,
    /// Only users on this node.
    pub node_id: Option<Uuid>,
    /// Text to find in the login name, email address or system name.
    pub q: Option<String>,
}

/// List users, newest first.
#[utoipa::path(
    get,
    path = "/api/v1/admin/users",
    tag = "users",
    operation_id = "listUsers",
    security(("session" = [])),
    params(ListQuery),
    responses(
        (status = 200, description = "A page of users", body = UserList),
        (status = 403, description = "`permission_denied` (needs admin.users.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid paging or filter", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_users(
    State(state): State<UsersState>,
    _who: Require<UsersRead>,
    Query(q): Query<ListQuery>,
) -> Result<Json<UserList>, AppError> {
    let mut errors = Vec::new();
    let mut number = |name: &'static str, text: &Option<String>, default, min, max| match text
        .as_deref()
        .filter(|t| !t.is_empty())
    {
        None => default,
        Some(t) => match t.parse::<i64>() {
            Ok(n) if (min..=max).contains(&n) => n,
            _ => {
                errors.push(FieldError::new(
                    name,
                    "out_of_range",
                    format!("Expected a number from {min} to {max}."),
                ));
                default
            }
        },
    };
    let limit = number("limit", &q.limit, DEFAULT_LIMIT, 1, MAX_LIMIT);
    let offset = number("offset", &q.offset, 0, 0, 10_000_000);
    let status = match q.status.as_deref().filter(|s| !s.is_empty()) {
        None => None,
        Some(s) => match TenantStatus::parse(s) {
            Ok(status) => Some(status),
            Err(_) => {
                errors.push(FieldError::new(
                    "status",
                    "unknown_status",
                    "Expected provisioning, active, suspended, deleting or failed.",
                ));
                None
            }
        },
    };
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let search = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if search.is_some_and(|s| s.chars().count() > 100) {
        return Err(AppError::validation(vec![FieldError::new(
            "q",
            "too_long",
            "At most 100 characters.",
        )]));
    }
    let (rows, total) = users::list(
        &state.db,
        &Filter {
            status,
            plan_id: q.plan_id,
            node_id: q.node_id,
            search,
        },
        limit,
        offset,
    )
    .await
    .map_err(internal)?;
    let ids: Vec<Uuid> = rows
        .iter()
        .filter_map(|u| u.tenant.as_ref().map(|t| t.id))
        .collect();
    let shares = builtpanel_database::usage::shares(&state.db, &ids)
        .await
        .map_err(internal)?;
    let items = rows
        .into_iter()
        .map(|user| {
            let mut v = view(user);
            if let Some(t) = v.tenant.as_mut() {
                let s = shares.get(&t.id).copied().unwrap_or_default();
                t.usage = Some(UsageShares {
                    memory_percent: s.memory,
                    cpu_percent: s.cpu,
                    disk_percent: s.disk,
                    inodes_percent: s.inodes,
                });
            }
            v
        })
        .collect();
    Ok(Json(UserList { items, total }))
}

/// One user.
#[utoipa::path(
    get,
    path = "/api/v1/admin/users/{id}",
    tag = "users",
    operation_id = "getUser",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 200, description = "The user", body = UserView),
        (status = 403, description = "`permission_denied` (needs admin.users.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_user(
    State(state): State<UsersState>,
    _who: Require<UsersRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserView>, AppError> {
    Ok(Json(view(load(&state, id).await?)))
}

// --------------------------------------------------------------------- create

/// A new user.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUser {
    /// Login name: 3-32 characters, a-z, 0-9, `_` or `-`, starting with a letter.
    #[schema(example = "alice")]
    pub username: String,
    /// Contact email address.
    pub email: String,
    /// Initial password; must satisfy the password policy.
    #[schema(format = Password)]
    pub password: String,
    /// The plan; the default plan when not given.
    pub plan_id: Option<Uuid>,
    /// The node; the online node with the fewest users when not given.
    pub node_id: Option<Uuid>,
    /// System user name on the node (a-z, 0-9, 3-32 characters); the login name
    /// without `_` and `-` when not given.
    pub system_username: Option<String>,
    /// Interface language, `en` (default) or `bn`.
    pub locale: Option<String>,
}

/// Create a user: the account, its hosting account and the work that sets it
/// up, in one transaction.
#[utoipa::path(
    post,
    path = "/api/v1/admin/users",
    tag = "users",
    operation_id = "createUser",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = CreateUser,
    responses(
        (status = 201, description = "Created; the hosting account is `provisioning` until the operation succeeds", body = UserChange),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`username_taken`, `email_taken`, `system_username_taken`, `no_node_available` or `node_unavailable`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid input (one error per field) or an unknown plan", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_user(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Json(req): Json<CreateUser>,
) -> Result<(StatusCode, Json<UserChange>), AppError> {
    create_user_inner(&state, &who, req, None).await
}

/// Creates a user for the administrator, or, with `reseller`, for that reseller: the
/// reseller's package limits are checked in the same transaction.
pub(crate) async fn create_user_inner(
    state: &UsersState,
    who: &Principal,
    req: CreateUser,
    reseller: Option<Uuid>,
) -> Result<(StatusCode, Json<UserChange>), AppError> {
    let mut errors = Vec::new();
    let username = Username::parse(&req.username)
        .map_err(|e| errors.push(FieldError::new("username", e.code(), e.to_string())))
        .ok();
    let email = Email::parse(&req.email)
        .map_err(|e| errors.push(FieldError::new("email", e.code(), e.to_string())))
        .ok();
    let system_name = req
        .system_username
        .clone()
        .unwrap_or_else(|| derived_tenant_name(&req.username));
    let system = TenantUsername::parse(&system_name)
        .map_err(|e| errors.push(FieldError::new("system_username", e.code(), e.to_string())))
        .ok();
    let locale = req.locale.clone().unwrap_or_else(|| "en".to_owned());
    if !matches!(locale.as_str(), "en" | "bn") {
        errors.push(FieldError::new(
            "locale",
            "unsupported_locale",
            "Expected en or bn.",
        ));
    }
    let password = SecretString::from(req.password);
    if let (Some(u), Some(e)) = (&username, &email)
        && let Err(violations) = PasswordPolicy::default().check(&password, u.as_str(), e.as_str())
    {
        errors.extend(
            violations
                .iter()
                .map(|v| FieldError::new("password", v.code(), v.message())),
        );
    }
    let plan = match req.plan_id {
        Some(id) => plans::get(&state.db, id).await.map_err(internal)?,
        None => plans::get_default(&state.db).await.map_err(internal)?,
    };
    let Some(plan) = plan else {
        errors.push(FieldError::new(
            "plan_id",
            "unknown_plan",
            "There is no such plan.",
        ));
        return Err(AppError::validation(errors));
    };
    let (Some(username), Some(email), Some(system), true) =
        (username, email, system, errors.is_empty())
    else {
        return Err(AppError::validation(errors));
    };

    let hash = state.hasher.hash_async(password).await.map_err(internal)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    if let Some(reseller_id) = reseller {
        crate::reseller::check_room(&mut tx, reseller_id, &plan).await?;
    }
    let node_id = match req.node_id {
        Some(id) => id,
        None => users::pick_node(&mut tx)
            .await
            .map_err(internal)?
            .ok_or_else(|| {
                AppError::conflict("no_node_available", "No node is online to host this user.")
            })?,
    };
    let id = accounts::insert_account_in(
        &mut tx,
        &NewAccount {
            kind: AccountKind::Client,
            username: &username,
            email: &email,
            password_hash: &hash,
        },
    )
    .await
    .map_err(|e| match e {
        InsertAccountError::UsernameTaken => {
            AppError::conflict("username_taken", "This username is already taken.")
        }
        InsertAccountError::EmailTaken => {
            AppError::conflict("email_taken", "This email address is already in use.")
        }
        InsertAccountError::Database(db) => internal(db),
    })?;
    if locale != "en" {
        users::update(&mut tx, id, None, Some(&locale))
            .await
            .map_err(|e| internal(std::io::Error::other(e.to_string())))?;
    }
    match rbac::assign_role_in(&mut tx, id, CLIENT_ROLE, Some(who.account_id()))
        .await
        .map_err(internal)?
    {
        AssignRole::Assigned | AssignRole::AlreadyAssigned => {}
        AssignRole::UnknownRole => return Err(internal(std::io::Error::other("no client role"))),
    }
    // The license's limit on hosting accounts.
    let hosting_accounts = builtpanel_database::sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM tenants WHERE status <> 'deleting'",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if let Some(reached) =
        builtpanel_database::license::check_limit(&mut tx, "max_tenants", hosting_accounts + 1)
            .await
            .map_err(internal)?
    {
        return Err(AppError::conflict(
            "license_limit",
            format!(
                "This license allows {} hosting account(s). Remove one or upgrade the license.",
                reached.limit
            ),
        ));
    }
    let tenant = tenants::insert(
        &mut tx,
        &tenants::NewTenant {
            owner_account_id: id,
            node_id,
            plan_id: plan.id,
            username: system.as_str(),
        },
    )
    .await
    .map_err(|e| match e {
        CreateError::UsernameTaken => AppError::conflict(
            "system_username_taken",
            "This system user name is already in use.",
        ),
        CreateError::NoFreeUid => {
            AppError::conflict("node_full", "This node has no room for another user.")
        }
        CreateError::UnknownReference => AppError::conflict(
            "node_unavailable",
            "The chosen node or plan does not exist.",
        ),
        CreateError::BadOwner | CreateError::Database(_) => internal(e),
    })?;
    if let Some(reseller_id) = reseller {
        builtpanel_database::resellers::assign_tenant(&mut tx, tenant.id, reseller_id)
            .await
            .map_err(internal)?;
    }
    let files = skeleton_for_new_tenant(&mut tx).await?;
    let op = set_up_operation(tenant.id, system.as_str(), tenant.uid, &plan, files)?;
    let operation_id = queue(&mut tx, node_id, tenant.id, &op, who).await?;
    let action = if reseller.is_some() {
        "reseller.user_created"
    } else {
        "admin.user_created"
    };
    builtpanel_audit::insert(
        &mut *tx,
        &entry(action, who, id).metadata(serde_json::json!({
            "username": username.as_str(),
            "system_username": system.as_str(),
            "plan": plan.name,
            "node_id": node_id,
            "operation_id": operation_id,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((
        StatusCode::CREATED,
        Json(UserChange {
            user: view(load(state, id).await?),
            operation_id: Some(operation_id),
        }),
    ))
}

// --------------------------------------------------------------------- update

/// Changes to a user's account.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateUser {
    /// New email address.
    pub email: Option<String>,
    /// New interface language, `en` or `bn`.
    pub locale: Option<String>,
}

/// Change a user's email address or language.
#[utoipa::path(
    patch,
    path = "/api/v1/admin/users/{id}",
    tag = "users",
    operation_id = "updateUser",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UpdateUser,
    responses(
        (status = 200, description = "Updated", body = UserView),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`email_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Nothing to change, or invalid input", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn update_user(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUser>,
) -> Result<Json<UserView>, AppError> {
    let mut errors = Vec::new();
    if req.email.is_none() && req.locale.is_none() {
        errors.push(FieldError::new(
            "email",
            "nothing_to_change",
            "Give an email address or a language to change.",
        ));
    }
    let email = req.email.as_deref().and_then(|e| {
        Email::parse(e)
            .map_err(|err| errors.push(FieldError::new("email", err.code(), err.to_string())))
            .ok()
    });
    if let Some(l) = &req.locale
        && !matches!(l.as_str(), "en" | "bn")
    {
        errors.push(FieldError::new(
            "locale",
            "unsupported_locale",
            "Expected en or bn.",
        ));
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    let found = users::update(&mut tx, id, email.as_ref(), req.locale.as_deref())
        .await
        .map_err(|e| match e {
            UpdateError::EmailTaken => {
                AppError::conflict("email_taken", "This email address is already in use.")
            }
            UpdateError::Database(db) => internal(db),
        })?;
    if !found {
        return Err(AppError::not_found("user"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &entry("admin.user_updated", &who, id).metadata(serde_json::json!({
            "email_changed": email.is_some(),
            "locale": req.locale,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(view(load(&state, id).await?)))
}

// ------------------------------------------------------------------ change plan

/// The plan to move a user to.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangePlan {
    /// The new plan.
    pub plan_id: Uuid,
}

/// Move a user to another plan; the node's limits are updated.
#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{id}/plan",
    tag = "users",
    operation_id = "changeUserPlan",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = ChangePlan,
    responses(
        (status = 200, description = "The plan was changed; `operation_id` follows the update of the limits", body = UserChange),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_tenant` or `tenant_busy` (only active or suspended users can change plan)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`unknown_plan`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn change_plan(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangePlan>,
) -> Result<Json<UserChange>, AppError> {
    let user = load(&state, id).await?;
    let tenant = tenant_of(&user)?;
    if !matches!(
        tenant.status,
        TenantStatus::Active | TenantStatus::Suspended
    ) {
        return Err(AppError::conflict(
            "tenant_busy",
            "Only an active or suspended user can change plan.",
        ));
    }
    let plan = plans::get(&state.db, req.plan_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            AppError::validation(vec![FieldError::new(
                "plan_id",
                "unknown_plan",
                "There is no such plan.",
            )])
        })?;
    let w = wire(&plan)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    users::set_plan(&mut tx, tenant.id, plan.id)
        .await
        .map_err(internal)?;
    let limits = queue(
        &mut tx,
        tenant.node_id,
        tenant.id,
        &NodeOperation::SetResourceLimits {
            username: tenant.username.clone(),
            cpu_millis: w.cpu_millis,
            memory_bytes: w.memory_bytes,
            pids: w.pids,
        },
        &who,
    )
    .await?;
    // Storage limits only where the node can enforce them.
    if users::node_enforces_storage_quota(&mut tx, tenant.node_id)
        .await
        .map_err(internal)?
    {
        queue(
            &mut tx,
            tenant.node_id,
            tenant.id,
            &NodeOperation::SetStorageQuota {
                tenant_id: tenant.id,
                uid: u32::try_from(tenant.uid)
                    .map_err(|_| internal(std::io::Error::other("negative uid")))?,
                disk_bytes: w.disk_bytes,
                inodes: w.inodes,
            },
            &who,
        )
        .await?;
    }
    builtpanel_audit::insert(
        &mut *tx,
        &entry("admin.user_plan_changed", &who, id).metadata(serde_json::json!({
            "from": tenant.plan_name,
            "to": plan.name,
            "operation_id": limits,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(UserChange {
        user: view(load(&state, id).await?),
        operation_id: Some(limits),
    }))
}

// --------------------------------------------------------------- suspend etc.

/// Suspend a user: the account cannot log in, the sites stop, the files stay.
#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{id}/suspend",
    tag = "users",
    operation_id = "suspendUser",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Suspended; `operation_id` follows the stopping of the stack", body = UserChange),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_tenant` or `tenant_busy` (only an active user can be suspended)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn suspend_user(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserChange>, AppError> {
    suspend_user_inner(&state, &who, id, false).await
}

pub(crate) async fn suspend_user_inner(
    state: &UsersState,
    who: &Principal,
    id: Uuid,
    by_reseller: bool,
) -> Result<Json<UserChange>, AppError> {
    let user = load(state, id).await?;
    let tenant = tenant_of(&user)?;
    if tenant.status != TenantStatus::Active {
        return Err(AppError::conflict(
            "tenant_busy",
            "Only an active user can be suspended.",
        ));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    users::set_account_status(&mut tx, id, "suspended")
        .await
        .map_err(internal)?;
    let operation_id = queue(
        &mut tx,
        tenant.node_id,
        tenant.id,
        &NodeOperation::SuspendTenant {
            username: tenant.username.clone(),
        },
        who,
    )
    .await?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry(
            if by_reseller {
                "reseller.user_suspended"
            } else {
                "admin.user_suspended"
            },
            who,
            id,
        )
        .metadata(serde_json::json!({ "operation_id": operation_id })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    end_sessions(state, id).await;
    Ok(Json(UserChange {
        user: view(load(state, id).await?),
        operation_id: Some(operation_id),
    }))
}

/// Lift a suspension: the account may log in again and the stack starts.
#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{id}/unsuspend",
    tag = "users",
    operation_id = "unsuspendUser",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Resumed; `operation_id` follows the starting of the stack", body = UserChange),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_tenant` or `tenant_busy` (only a suspended user can be resumed)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn unsuspend_user(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserChange>, AppError> {
    unsuspend_user_inner(&state, &who, id, false).await
}

pub(crate) async fn unsuspend_user_inner(
    state: &UsersState,
    who: &Principal,
    id: Uuid,
    by_reseller: bool,
) -> Result<Json<UserChange>, AppError> {
    let user = load(state, id).await?;
    let tenant = tenant_of(&user)?;
    if tenant.status != TenantStatus::Suspended {
        return Err(AppError::conflict(
            "tenant_busy",
            "Only a suspended user can be resumed.",
        ));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    users::set_account_status(&mut tx, id, "active")
        .await
        .map_err(internal)?;
    let operation_id = queue(
        &mut tx,
        tenant.node_id,
        tenant.id,
        &NodeOperation::ResumeTenant {
            username: tenant.username.clone(),
        },
        who,
    )
    .await?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry(
            if by_reseller {
                "reseller.user_unsuspended"
            } else {
                "admin.user_unsuspended"
            },
            who,
            id,
        )
        .metadata(serde_json::json!({ "operation_id": operation_id })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(UserChange {
        user: view(load(state, id).await?),
        operation_id: Some(operation_id),
    }))
}

/// Delete a user: the account is disabled and the stack is removed at once; the
/// files are kept until `purge_after`, then purged.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/users/{id}",
    tag = "users",
    operation_id = "deleteUser",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Deleted (soft); `operation_id` follows the removal of the stack", body = UserChange),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`tenant_busy` (already being deleted)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_user(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserChange>, AppError> {
    delete_user_inner(&state, &who, id, false).await
}

pub(crate) async fn delete_user_inner(
    state: &UsersState,
    who: &Principal,
    id: Uuid,
    by_reseller: bool,
) -> Result<Json<UserChange>, AppError> {
    let user = load(state, id).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let mut operation_id = None;
    if let Some(tenant) = &user.tenant {
        if !tenants::set_status(
            &mut tx,
            tenant.id,
            TenantStatus::Deleting,
            state.purge_after_days,
        )
        .await
        .map_err(internal)?
        {
            return Err(AppError::conflict(
                "tenant_busy",
                "This user is already being deleted.",
            ));
        }
        operation_id = Some(
            queue(
                &mut tx,
                tenant.node_id,
                tenant.id,
                &NodeOperation::DeleteTenant {
                    tenant_id: tenant.id,
                    username: tenant.username.clone(),
                    uid: u32::try_from(tenant.uid)
                        .map_err(|_| internal(std::io::Error::other("negative uid")))?,
                    purge_data: false,
                },
                who,
            )
            .await?,
        );
    }
    users::set_account_status(&mut tx, id, "disabled")
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry(
            if by_reseller {
                "reseller.user_deleted"
            } else {
                "admin.user_deleted"
            },
            who,
            id,
        )
        .metadata(serde_json::json!({
            "operation_id": operation_id,
            "purge_after_days": state.purge_after_days,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    end_sessions(state, id).await;
    Ok(Json(UserChange {
        user: view(load(state, id).await?),
        operation_id,
    }))
}

/// Run the set-up of a user whose hosting account is `failed` again.
#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{id}/retry-setup",
    tag = "users",
    operation_id = "retryUserSetup",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Queued again; the hosting account is `provisioning`", body = UserChange),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`no_tenant` or `tenant_busy` (only a failed set-up can be retried)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn retry_setup(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserChange>, AppError> {
    let user = load(&state, id).await?;
    let tenant = tenant_of(&user)?;
    if tenant.status != TenantStatus::Failed {
        return Err(AppError::conflict(
            "tenant_busy",
            "Only a user whose set-up failed can be set up again.",
        ));
    }
    let plan = plans::get(&state.db, tenant.plan_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("the plan of a tenant vanished")))?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !tenants::set_status(&mut tx, tenant.id, TenantStatus::Provisioning, 0)
        .await
        .map_err(internal)?
    {
        return Err(AppError::conflict(
            "tenant_busy",
            "Only a user whose set-up failed can be set up again.",
        ));
    }
    let files = skeleton_for_new_tenant(&mut tx).await?;
    let op = set_up_operation(tenant.id, &tenant.username, tenant.uid, &plan, files)?;
    let operation_id = queue(&mut tx, tenant.node_id, tenant.id, &op, &who).await?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("admin.user_setup_retried", &who, id)
            .metadata(serde_json::json!({ "operation_id": operation_id })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(UserChange {
        user: view(load(&state, id).await?),
        operation_id: Some(operation_id),
    }))
}

// -------------------------------------------------------------------- password

/// A new password for a user.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResetPassword {
    /// The new password; must satisfy the password policy.
    #[schema(format = Password)]
    pub password: String,
}

/// Set a new password for a user and sign them out everywhere.
#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{id}/password",
    tag = "users",
    operation_id = "resetUserPassword",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = ResetPassword,
    responses(
        (status = 204, description = "Changed; every session of the user ended"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The password breaks the policy", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn reset_password(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<ResetPassword>,
) -> Result<StatusCode, AppError> {
    let user = load(&state, id).await?;
    let password = SecretString::from(req.password);
    if let Err(violations) = PasswordPolicy::default().check(&password, &user.username, &user.email)
    {
        return Err(AppError::validation(
            violations
                .iter()
                .map(|v| FieldError::new("password", v.code(), v.message()))
                .collect(),
        ));
    }
    let hash = state.hasher.hash_async(password).await.map_err(internal)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    users::set_password_hash(&mut tx, id, &hash)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(&mut *tx, &entry("admin.user_password_reset", &who, id))
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    end_sessions(&state, id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Ends every session of the user. The account change is already committed, so a
/// failure here is logged and left for the session's own expiry rather than
/// undoing the change.
pub(crate) async fn end_sessions(state: &UsersState, id: Uuid) {
    if state
        .sessions
        .revoke_all(id, None, RevokeReason::Admin)
        .await
        .is_err()
    {
        eprintln!("users: could not end the sessions of {id}");
    }
}

/// A one-time sign-in token for the client app.
#[derive(Serialize, ToSchema)]
pub struct ImpersonationToken {
    /// Give it to the client app; it is shown once and works once.
    pub token: String,
    /// Seconds the token stays valid.
    pub expires_in: u64,
    /// Where the client app spends it: `POST` it as `{"token": ...}`.
    pub redeem_path: &'static str,
}

/// Get a one-time token to sign in to the client app as this user ("login as
/// user"). The session it opens shows who is behind it, never counts as
/// recently authenticated, and both names are in the audit log.
#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{id}/impersonate",
    tag = "users",
    operation_id = "impersonateUser",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "User id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "The token", body = ImpersonationToken),
        (status = 403, description = "`permission_denied` (needs admin.users.impersonate), `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such user", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`user_unavailable`: only an active user can be signed in as", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn impersonate(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersImpersonate>,
    Path(id): Path<Uuid>,
) -> Result<Json<ImpersonationToken>, AppError> {
    let user = load(&state, id).await?;
    if user.account_status != "active"
        || user
            .tenant
            .as_ref()
            .is_some_and(|t| t.status == TenantStatus::Deleting)
    {
        return Err(AppError::conflict(
            "user_unavailable",
            "Only an active user can be signed in as.",
        ));
    }
    let token = state
        .sessions
        .issue_impersonation(who.account_id(), id)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &state.db,
        &entry("admin.impersonation_issued", &who, id)
            .metadata(serde_json::json!({ "user": user.username })),
    )
    .await
    .map_err(internal)?;
    Ok(Json(ImpersonationToken {
        token: secrecy::ExposeSecret::expose_secret(&token).to_owned(),
        expires_in: builtpanel_session::IMPERSONATION_TTL_SECS,
        redeem_path: "/api/v1/public/auth/impersonate",
    }))
}

// ----------------------------------------------------------- skeleton files

/// A default file of new users.
#[derive(Debug, Serialize, ToSchema)]
pub struct SkeletonFileView {
    /// Path below the docroot, for example `index.html` or `css/site.css`.
    pub path: String,
    /// The file's text.
    pub content: String,
    /// Size in bytes.
    pub size: usize,
    /// Last change (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub updated_at: String,
}

/// The default files.
#[derive(Debug, Serialize, ToSchema)]
pub struct SkeletonFileList {
    /// By path.
    pub items: Vec<SkeletonFileView>,
    /// Bytes in all files (the limit is 24576).
    pub total_bytes: usize,
}

/// A default file to add or replace.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PutSkeletonFile {
    /// Path below the docroot: names of letters, digits, `.`, `_`, `-` separated
    /// by `/`, at most 6 levels.
    #[schema(example = "index.html")]
    pub path: String,
    /// The text, at most 16 KiB.
    pub content: String,
}

/// Which default file.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SkeletonPath {
    /// The file's path.
    pub path: String,
}

fn skeleton_list(files: Vec<skeleton::SkeletonFile>) -> SkeletonFileList {
    let total_bytes = files.iter().map(|f| f.content.len()).sum();
    SkeletonFileList {
        items: files
            .into_iter()
            .map(|f| SkeletonFileView {
                size: f.content.len(),
                updated_at: rfc3339(f.updated_at),
                path: f.path,
                content: f.content,
            })
            .collect(),
        total_bytes,
    }
}

/// The files every new user's docroot starts with.
#[utoipa::path(
    get,
    path = "/api/v1/admin/skeleton-files",
    tag = "users",
    operation_id = "listSkeletonFiles",
    security(("session" = [])),
    responses(
        (status = 200, description = "The default files", body = SkeletonFileList),
        (status = 403, description = "`permission_denied` (needs admin.users.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_skeleton_files(
    State(state): State<UsersState>,
    _who: Require<UsersRead>,
) -> Result<Json<SkeletonFileList>, AppError> {
    Ok(Json(skeleton_list(
        skeleton::list(&state.db).await.map_err(internal)?,
    )))
}

/// Add a default file or replace its content. Existing users keep their files.
#[utoipa::path(
    put,
    path = "/api/v1/admin/skeleton-files",
    tag = "users",
    operation_id = "putSkeletonFile",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = PutSkeletonFile,
    responses(
        (status = 200, description = "Saved; the whole list", body = SkeletonFileList),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A bad path, or the files would be too many or too large", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn put_skeleton_file(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Json(req): Json<PutSkeletonFile>,
) -> Result<Json<SkeletonFileList>, AppError> {
    // The list as it would be, checked the way the node will check it.
    let mut tx = state.db.begin().await.map_err(internal)?;
    let mut files: Vec<SkeletonFile> = skeleton::list_in(&mut tx)
        .await
        .map_err(internal)?
        .into_iter()
        .filter(|f| f.path != req.path)
        .map(|f| SkeletonFile {
            path: f.path,
            content: f.content,
        })
        .collect();
    files.push(SkeletonFile {
        path: req.path.clone(),
        content: req.content.clone(),
    });
    if let Err(message) = check_skeleton(&files) {
        return Err(AppError::validation(vec![FieldError::new(
            "path",
            "invalid_skeleton",
            message,
        )]));
    }
    skeleton::put(&mut tx, &req.path, &req.content, who.account_id())
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("admin.skeleton_file_saved", Outcome::Success)
            .actor(who.account_id())
            .target("skeleton_file", req.path.as_str())
            .metadata(serde_json::json!({ "size": req.content.len() })),
    )
    .await
    .map_err(internal)?;
    let list = skeleton::list_in(&mut tx).await.map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(skeleton_list(list)))
}

/// Remove a default file. Existing users keep their files.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/skeleton-files",
    tag = "users",
    operation_id = "deleteSkeletonFile",
    security(("session" = [])),
    params(
        SkeletonPath,
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, description = "`permission_denied`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such file", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_skeleton_file(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<UsersManage>,
    Query(q): Query<SkeletonPath>,
) -> Result<StatusCode, AppError> {
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !skeleton::delete(&mut tx, &q.path).await.map_err(internal)? {
        return Err(AppError::not_found("skeleton file"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("admin.skeleton_file_deleted", Outcome::Success)
            .actor(who.account_id())
            .target("skeleton_file", q.path.as_str()),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// OpenAPI fragment (full paths; merge into the root document).
#[derive(OpenApi)]
#[openapi(
    paths(
        list_users, get_user, create_user, update_user, change_plan, suspend_user,
        unsuspend_user, delete_user, retry_setup, reset_password, impersonate,
        list_skeleton_files, put_skeleton_file, delete_skeleton_file
    ),
    components(schemas(
        TenantView, UsageShares, UserView, UserChange, UserList, CreateUser, UpdateUser, ChangePlan,
        ResetPassword, ImpersonationToken, SkeletonFileView, SkeletonFileList, PutSkeletonFile
    )),
    tags((name = "users", description = "Client accounts and their hosting"))
)]
pub struct UsersApi;

/// OpenAPI description of the resellers module.
#[derive(OpenApi)]
#[openapi(
    paths(
        reseller::list_packages, reseller::create_package, reseller::delete_package,
        reseller::list_resellers, reseller::create_reseller, reseller::set_reseller_status,
        reseller::delete_reseller, reseller::me, reseller::my_package, reseller::put_branding,
        reseller::my_users, reseller::create_my_user, reseller::set_my_user_status,
        reseller::delete_my_user, reseller::my_plans, reseller::create_my_plan,
        reseller::delete_my_plan
    ),
    components(schemas(
        reseller::PackageView, reseller::CreatePackage, reseller::BrandingView,
        reseller::ResellerView, reseller::ClientView, reseller::ResellerPlanView,
        reseller::CreateReseller, reseller::SetStatus, reseller::PutBranding,
        reseller::CreateClient, reseller::CreatePlan
    )),
    tags((name = "resellers", description = "Resellers, their packages, clients and plans"))
)]
pub struct ResellersApi;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_names_are_derived_from_login_names() {
        assert_eq!(derived_tenant_name("alice"), "alice");
        assert_eq!(derived_tenant_name("al-ice_99"), "alice99");
        assert!(TenantUsername::parse(&derived_tenant_name("a-b")).is_err());
    }
}
