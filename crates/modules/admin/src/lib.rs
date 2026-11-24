//! Administrator management.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET    | `/api/v1/admin/administrators` | `admin.administrators.read` |
//! | GET    | `/api/v1/admin/administrators/{id}` | `admin.administrators.read` |
//! | POST   | `/api/v1/admin/administrators` | super administrator, sudo mode |
//! | PATCH  | `/api/v1/admin/administrators/{id}` | super administrator, sudo mode |
//! | DELETE | `/api/v1/admin/administrators/{id}` | super administrator, sudo mode |
//! | PUT    | `/api/v1/admin/administrators/{id}/roles/{role}` | super administrator, sudo mode |
//! | DELETE | `/api/v1/admin/administrators/{id}/roles/{role}` | super administrator, sudo mode |
//! | POST   | `/api/v1/admin/administrators/{id}/mfa/reset` | super administrator, sudo mode |
//! | DELETE | `/api/v1/admin/administrators/{id}/sessions` | super administrator, sudo mode |
//!
//! Rules:
//! * Only a **super administrator** (role `super_admin`) can create, change or
//!   remove administrators or their roles, on top of holding the
//!   `admin.administrators.manage` permission. Other roles get 403.
//! * Every change needs sudo mode (a recent password confirmation), and is
//!   recorded in the audit log in the **same transaction** as the change.
//! * The last active super administrator can never be removed, suspended or
//!   demoted, and nobody can suspend or delete themselves. Changes are
//!   serialised with a database lock, so two simultaneous requests cannot
//!   both pass the check.
//! * New administrators must use two-factor authentication unless the request
//!   says otherwise; until they enrol, permission-protected endpoints answer
//!   403 `mfa_setup_required`.
//! * `client` and `reseller` roles belong to other account kinds and cannot be
//!   given to administrators.

use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_auth_core::password::PasswordHasher;
use builtpanel_auth_core::policy::PasswordPolicy;
use builtpanel_database::{
    accounts::{self, DbAccountStatus, InsertAccountError, NewAccount},
    admins::{self, AdminRow, SUPER_ADMIN_ROLE, UpdateAdminError},
    rbac::{self, AssignRole},
    sqlx::{PgConnection, PgPool, Postgres, Transaction},
};
use builtpanel_domain::{AccountKind, Email, Username};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{AdministratorsManage, AdministratorsRead, RolesRead},
};
use builtpanel_session::{RevokeReason, SessionStore, rfc3339};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

/// Role given to a new administrator when none is requested.
pub const DEFAULT_ROLE: &str = "admin";
/// Roles that belong to other kinds of account.
const NOT_FOR_ADMINISTRATORS: [&str; 2] = ["client", "reseller"];
/// Entries per page when `limit` is not given.
pub const DEFAULT_LIMIT: i64 = 50;
/// Most entries per page.
pub const MAX_LIMIT: i64 = 100;

/// Dependencies of the admin handlers.
#[derive(Clone)]
pub struct AdminState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// Password hasher with the configured cost.
    pub hasher: PasswordHasher,
}

impl FromRef<AdminState> for PgPool {
    fn from_ref(s: &AdminState) -> Self {
        s.db.clone()
    }
}

impl FromRef<AdminState> for SessionStore {
    fn from_ref(s: &AdminState) -> Self {
        s.sessions.clone()
    }
}

/// Routes with full paths; merge into the application router.
pub fn router() -> Router<AdminState> {
    Router::new()
        .route(
            "/api/v1/admin/administrators",
            get(list_administrators).post(create_administrator),
        )
        .route(
            "/api/v1/admin/administrators/{id}",
            get(get_administrator)
                .patch(update_administrator)
                .delete(delete_administrator),
        )
        .route(
            "/api/v1/admin/administrators/{id}/roles/{role}",
            put(assign_administrator_role).delete(revoke_administrator_role),
        )
        .route(
            "/api/v1/admin/administrators/{id}/mfa/reset",
            post(reset_administrator_mfa),
        )
        .route(
            "/api/v1/admin/administrators/{id}/sessions",
            axum::routing::delete(end_administrator_sessions),
        )
        .route("/api/v1/admin/permissions", get(list_permissions))
}

// ------------------------------------------------------------------ types

/// An administrator account.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdministratorView {
    /// Account id.
    pub id: Uuid,
    /// Login name.
    pub username: String,
    /// Email address.
    pub email: String,
    /// `active`, `suspended` or `disabled`.
    #[schema(example = "active")]
    pub status: String,
    /// Role names.
    pub roles: Vec<String>,
    /// A second factor is set up.
    pub mfa_enabled: bool,
    /// Two-factor authentication is mandatory for this account.
    pub mfa_required: bool,
    /// When the account was created (RFC 3339, UTC).
    #[schema(format = DateTime, example = "2026-11-11T09:30:00Z")]
    pub created_at: String,
    /// Last successful login (RFC 3339, UTC), if any.
    #[schema(format = DateTime)]
    pub last_login_at: Option<String>,
}

fn status_name(status: DbAccountStatus) -> &'static str {
    match status {
        DbAccountStatus::Active => "active",
        DbAccountStatus::Suspended => "suspended",
        DbAccountStatus::Disabled => "disabled",
    }
}

impl From<AdminRow> for AdministratorView {
    fn from(r: AdminRow) -> Self {
        Self {
            id: r.id,
            username: r.username,
            email: r.email,
            status: status_name(r.status).to_owned(),
            roles: r.roles,
            mfa_enabled: r.mfa_enabled,
            mfa_required: r.mfa_required,
            created_at: rfc3339(r.created_at),
            last_login_at: r.last_login_at.map(rfc3339),
        }
    }
}

/// A page of administrators.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdministratorList {
    /// Oldest account first.
    pub items: Vec<AdministratorView>,
    /// Pass as `after` to get the next page; absent on the last page.
    pub next_after: Option<Uuid>,
}

/// Paging for the list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    /// Entries per page, 1..=100 (default 50).
    pub limit: Option<String>,
    /// `next_after` of the previous page.
    pub after: Option<String>,
}

/// New administrator.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAdministrator {
    /// Login name: 3-32 characters, a-z, 0-9, `_` or `-`, starting with a letter.
    #[schema(example = "khairul")]
    pub username: String,
    /// Contact email address.
    pub email: String,
    /// Initial password; must satisfy the password policy.
    #[schema(format = Password)]
    pub password: String,
    /// Role to grant (default `admin`).
    #[serde(default)]
    #[schema(example = "admin")]
    pub role: Option<String>,
    /// Require two-factor authentication (default `true`).
    #[serde(default)]
    pub require_mfa: Option<bool>,
}

impl std::fmt::Debug for CreateAdministrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateAdministrator")
            .field("username", &self.username)
            .finish_non_exhaustive()
    }
}

/// Status an administrator can be set to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum StatusInput {
    /// May log in.
    Active,
    /// Temporarily blocked; sessions are ended.
    Suspended,
    /// Permanently blocked; sessions are ended.
    Disabled,
}

impl From<StatusInput> for DbAccountStatus {
    fn from(s: StatusInput) -> Self {
        match s {
            StatusInput::Active => Self::Active,
            StatusInput::Suspended => Self::Suspended,
            StatusInput::Disabled => Self::Disabled,
        }
    }
}

/// Changes to an administrator; omitted fields stay as they are.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAdministrator {
    /// New email address.
    pub email: Option<String>,
    /// New status.
    pub status: Option<StatusInput>,
    /// Make two-factor authentication mandatory (or not).
    pub require_mfa: Option<bool>,
}

/// Result of ending sessions.
#[derive(Debug, Serialize, ToSchema)]
pub struct EndedSessions {
    /// Number of sessions ended.
    pub revoked: u64,
}

// ---------------------------------------------------------------- helpers

fn parse_admin_id(id: &str) -> Result<Uuid, AppError> {
    id.parse().map_err(|_| AppError::not_found("administrator"))
}

/// The hard rule: managing administrators is for super administrators only,
/// whatever other permissions a custom role may carry.
fn super_admin_only(who: &Principal) -> Result<(), AppError> {
    if who.has_role(SUPER_ADMIN_ROLE) {
        Ok(())
    } else {
        Err(AppError::forbidden(
            "super_admin_required",
            "Only a super administrator can manage administrators.",
        ))
    }
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

fn assignable(role: &str) -> bool {
    !NOT_FOR_ADMINISTRATORS.contains(&role)
}

fn entry(action: &'static str, who: &Principal, target: Uuid) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target("administrator", target)
}

/// Starts a transaction that holds the administrator-changes lock.
async fn begin_locked(db: &PgPool) -> Result<Transaction<'static, Postgres>, AppError> {
    let mut tx = db.begin().await.map_err(internal)?;
    admins::lock_admin_changes(&mut tx)
        .await
        .map_err(internal)?;
    Ok(tx)
}

async fn load_target(conn: &mut PgConnection, id: Uuid) -> Result<AdminRow, AppError> {
    admins::find_admin(&mut *conn, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("administrator"))
}

/// Refuses a change that would leave the panel without an active super administrator.
async fn keep_one_super_admin(conn: &mut PgConnection, target: &AdminRow) -> Result<(), AppError> {
    if target.is_active_super_admin()
        && admins::count_active_super_admins(&mut *conn)
            .await
            .map_err(internal)?
            <= 1
    {
        return Err(AppError::conflict(
            "last_super_admin",
            "This is the last active super administrator; make another one first.",
        ));
    }
    Ok(())
}

/// Ends all of an account's sessions; a failure is logged, never fatal,
/// because the account is already blocked by then.
async fn end_sessions(state: &AdminState, account_id: Uuid, reason: RevokeReason) -> u64 {
    match state.sessions.revoke_all(account_id, None, reason).await {
        Ok(n) => n,
        Err(err) => {
            tracing::error!(%account_id, error = %err, "could not end the sessions of a changed administrator");
            0
        }
    }
}

async fn reload(state: &AdminState, id: Uuid) -> Result<AdministratorView, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    admins::find_admin(&mut conn, id)
        .await
        .map_err(internal)?
        .map(AdministratorView::from)
        .ok_or_else(|| AppError::not_found("administrator"))
}

// --------------------------------------------------------------- handlers

/// List administrators.
#[utoipa::path(
    get,
    path = "/api/v1/admin/administrators",
    tag = "administrators",
    operation_id = "listAdministrators",
    security(("session" = [])),
    params(ListQuery),
    responses(
        (status = 200, description = "A page of administrators", body = AdministratorList),
        (status = 403, description = "`permission_denied` (needs admin.administrators.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid paging", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_administrators(
    State(state): State<AdminState>,
    _who: Require<AdministratorsRead>,
    Query(q): Query<ListQuery>,
) -> Result<Json<AdministratorList>, AppError> {
    let mut errors = Vec::new();
    let mut limit = DEFAULT_LIMIT;
    if let Some(l) = q.limit.filter(|l| !l.is_empty()) {
        match l.parse::<i64>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => limit = n,
            _ => errors.push(FieldError::new(
                "limit",
                "out_of_range",
                format!("Expected a number from 1 to {MAX_LIMIT}."),
            )),
        }
    }
    let mut after = None;
    if let Some(a) = q.after.filter(|a| !a.is_empty()) {
        match a.parse::<Uuid>() {
            Ok(id) => after = Some(id),
            Err(_) => errors.push(FieldError::new(
                "after",
                "invalid_format",
                "Expected a UUID.",
            )),
        }
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }

    let mut rows = admins::list_admins(&state.db, after, limit + 1)
        .await
        .map_err(internal)?;
    let has_more = i64::try_from(rows.len()).is_ok_and(|n| n > limit);
    if has_more {
        rows.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    let next_after = has_more.then(|| rows.last().map(|r| r.id)).flatten();
    Ok(Json(AdministratorList {
        items: rows.into_iter().map(AdministratorView::from).collect(),
        next_after,
    }))
}

/// Show one administrator.
#[utoipa::path(
    get,
    path = "/api/v1/admin/administrators/{id}",
    tag = "administrators",
    operation_id = "getAdministrator",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Administrator id")),
    responses(
        (status = 200, description = "The administrator", body = AdministratorView),
        (status = 403, description = "`permission_denied` or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such administrator", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn get_administrator(
    State(state): State<AdminState>,
    _who: Require<AdministratorsRead>,
    Path(id): Path<String>,
) -> Result<Json<AdministratorView>, AppError> {
    Ok(Json(reload(&state, parse_admin_id(&id)?).await?))
}

/// Create an administrator.
#[utoipa::path(
    post,
    path = "/api/v1/admin/administrators",
    tag = "administrators",
    operation_id = "createAdministrator",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = CreateAdministrator,
    responses(
        (status = 201, description = "Created", body = AdministratorView),
        (status = 403, description = "`permission_denied`, `super_admin_required`, `reauth_required` (call POST /api/v1/auth/sudo) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`username_taken` or `email_taken`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid input; one error per field", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_administrator(
    State(state): State<AdminState>,
    RequireRecent(who, _): RequireRecent<AdministratorsManage>,
    Json(req): Json<CreateAdministrator>,
) -> Result<(StatusCode, Json<AdministratorView>), AppError> {
    super_admin_only(&who)?;

    let mut errors = Vec::new();
    let username = Username::parse(&req.username)
        .map_err(|e| errors.push(FieldError::new("username", e.code(), e.to_string())))
        .ok();
    let email = Email::parse(&req.email)
        .map_err(|e| errors.push(FieldError::new("email", e.code(), e.to_string())))
        .ok();
    let role = req.role.unwrap_or_else(|| DEFAULT_ROLE.to_owned());
    if !assignable(&role) {
        errors.push(FieldError::new(
            "role",
            "role_not_assignable",
            "This role belongs to other kinds of account.",
        ));
    } else if !rbac::role_exists(&state.db, &role)
        .await
        .map_err(internal)?
    {
        errors.push(FieldError::new(
            "role",
            "unknown_role",
            "There is no such role.",
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
    let (Some(username), Some(email), true) = (username, email, errors.is_empty()) else {
        return Err(AppError::validation(errors));
    };

    let hash = state.hasher.hash_async(password).await.map_err(internal)?;
    let require_mfa = req.require_mfa.unwrap_or(true);

    let mut tx = begin_locked(&state.db).await?;
    let id = accounts::insert_account_in(
        &mut tx,
        &NewAccount {
            kind: AccountKind::Admin,
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
    match rbac::assign_role_in(&mut tx, id, &role, Some(who.account_id()))
        .await
        .map_err(internal)?
    {
        AssignRole::Assigned | AssignRole::AlreadyAssigned => {}
        AssignRole::UnknownRole => return Err(internal(std::io::Error::other("role vanished"))),
    }
    admins::update_admin(&mut tx, id, None, None, Some(require_mfa))
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("admin.administrator_created", &who, id).metadata(serde_json::json!({
            "username": username.as_str(),
            "role": role,
            "require_mfa": require_mfa,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;

    Ok((StatusCode::CREATED, Json(reload(&state, id).await?)))
}

/// Change an administrator's email, status or two-factor requirement.
#[utoipa::path(
    patch,
    path = "/api/v1/admin/administrators/{id}",
    tag = "administrators",
    operation_id = "updateAdministrator",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Administrator id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = UpdateAdministrator,
    responses(
        (status = 200, description = "Updated", body = AdministratorView),
        (status = 403, description = "`permission_denied`, `super_admin_required`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such administrator", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`email_taken`, `cannot_lock_yourself` or `last_super_admin`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Nothing to change, or invalid input", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn update_administrator(
    State(state): State<AdminState>,
    RequireRecent(who, _): RequireRecent<AdministratorsManage>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAdministrator>,
) -> Result<Json<AdministratorView>, AppError> {
    super_admin_only(&who)?;
    let id = parse_admin_id(&id)?;
    if req.email.is_none() && req.status.is_none() && req.require_mfa.is_none() {
        return Err(AppError::validation(vec![FieldError::new(
            "body",
            "empty_update",
            "Give at least one of email, status or require_mfa.",
        )]));
    }
    let email = match &req.email {
        Some(e) => Some(Email::parse(e).map_err(|err| {
            AppError::validation(vec![FieldError::new("email", err.code(), err.to_string())])
        })?),
        None => None,
    };
    let status: Option<DbAccountStatus> = req.status.map(Into::into);

    let mut tx = begin_locked(&state.db).await?;
    let target = load_target(&mut tx, id).await?;
    let blocks = status.is_some_and(|s| s != DbAccountStatus::Active);
    if blocks && id == who.account_id() {
        return Err(AppError::conflict(
            "cannot_lock_yourself",
            "You cannot suspend or disable your own account.",
        ));
    }
    if blocks {
        keep_one_super_admin(&mut tx, &target).await?;
    }
    admins::update_admin(
        &mut tx,
        id,
        email.as_ref().map(Email::as_str),
        status,
        req.require_mfa,
    )
    .await
    .map_err(|e| match e {
        UpdateAdminError::EmailTaken => {
            AppError::conflict("email_taken", "This email address is already in use.")
        }
        UpdateAdminError::Database(db) => internal(db),
    })?;
    let mut changed = Vec::new();
    if email.is_some() {
        changed.push("email");
    }
    if status.is_some() {
        changed.push("status");
    }
    if req.require_mfa.is_some() {
        changed.push("require_mfa");
    }
    let mut metadata = serde_json::json!({ "changed": changed });
    if let Some(s) = status {
        metadata["status"] = status_name(s).into();
    }
    if let Some(r) = req.require_mfa {
        metadata["require_mfa"] = r.into();
    }
    builtpanel_audit::insert(
        &mut *tx,
        &entry("admin.administrator_updated", &who, id).metadata(metadata),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;

    if blocks {
        end_sessions(&state, id, RevokeReason::Admin).await;
    }
    Ok(Json(reload(&state, id).await?))
}

/// Delete an administrator. Their sessions end; audit entries stay.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/administrators/{id}",
    tag = "administrators",
    operation_id = "deleteAdministrator",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Administrator id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "`permission_denied`, `super_admin_required`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such administrator", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`cannot_delete_self` or `last_super_admin`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_administrator(
    State(state): State<AdminState>,
    RequireRecent(who, _): RequireRecent<AdministratorsManage>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    super_admin_only(&who)?;
    let id = parse_admin_id(&id)?;
    if id == who.account_id() {
        return Err(AppError::conflict(
            "cannot_delete_self",
            "You cannot delete your own account.",
        ));
    }
    let mut tx = begin_locked(&state.db).await?;
    let target = load_target(&mut tx, id).await?;
    keep_one_super_admin(&mut tx, &target).await?;
    // Their Redis sessions are not removed by the database cascade, so end
    // them first. This only touches the sessions table; the transaction has
    // not locked those rows yet.
    end_sessions(&state, id, RevokeReason::Admin).await;
    admins::delete_admin(&mut tx, id).await.map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("admin.administrator_deleted", &who, id)
            .metadata(serde_json::json!({ "username": target.username })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Give an administrator a role (no change when they already have it).
#[utoipa::path(
    put,
    path = "/api/v1/admin/administrators/{id}/roles/{role}",
    tag = "administrators",
    operation_id = "assignAdministratorRole",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Administrator id"),
        ("role" = String, Path, description = "Role name"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "The administrator with their roles", body = AdministratorView),
        (status = 403, description = "`permission_denied`, `super_admin_required`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such administrator or role", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`role_not_assignable`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn assign_administrator_role(
    State(state): State<AdminState>,
    RequireRecent(who, _): RequireRecent<AdministratorsManage>,
    Path((id, role)): Path<(String, String)>,
) -> Result<Json<AdministratorView>, AppError> {
    super_admin_only(&who)?;
    let id = parse_admin_id(&id)?;
    if !assignable(&role) {
        return Err(AppError::conflict(
            "role_not_assignable",
            "This role belongs to other kinds of account.",
        ));
    }
    let mut tx = begin_locked(&state.db).await?;
    load_target(&mut tx, id).await?;
    match rbac::assign_role_in(&mut tx, id, &role, Some(who.account_id()))
        .await
        .map_err(internal)?
    {
        AssignRole::UnknownRole => return Err(AppError::not_found("role")),
        AssignRole::AlreadyAssigned => {}
        AssignRole::Assigned => {
            builtpanel_audit::insert(
                &mut *tx,
                &entry("admin.role_assigned", &who, id)
                    .metadata(serde_json::json!({ "role": role })),
            )
            .await
            .map_err(internal)?;
        }
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(reload(&state, id).await?))
}

/// Take a role away from an administrator (no change when they do not have it).
#[utoipa::path(
    delete,
    path = "/api/v1/admin/administrators/{id}/roles/{role}",
    tag = "administrators",
    operation_id = "revokeAdministratorRole",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Administrator id"),
        ("role" = String, Path, description = "Role name"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "The administrator with their roles", body = AdministratorView),
        (status = 403, description = "`permission_denied`, `super_admin_required`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such administrator", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`last_super_admin`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn revoke_administrator_role(
    State(state): State<AdminState>,
    RequireRecent(who, _): RequireRecent<AdministratorsManage>,
    Path((id, role)): Path<(String, String)>,
) -> Result<Json<AdministratorView>, AppError> {
    super_admin_only(&who)?;
    let id = parse_admin_id(&id)?;
    let mut tx = begin_locked(&state.db).await?;
    let target = load_target(&mut tx, id).await?;
    if role == SUPER_ADMIN_ROLE {
        keep_one_super_admin(&mut tx, &target).await?;
    }
    if rbac::revoke_role_in(&mut tx, id, &role)
        .await
        .map_err(internal)?
    {
        builtpanel_audit::insert(
            &mut *tx,
            &entry("admin.role_revoked", &who, id).metadata(serde_json::json!({ "role": role })),
        )
        .await
        .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    Ok(Json(reload(&state, id).await?))
}

/// Remove an administrator's second factor and backup codes (lost phone) and
/// end their sessions. If 2FA is mandatory for them, they must enrol again.
#[utoipa::path(
    post,
    path = "/api/v1/admin/administrators/{id}/mfa/reset",
    tag = "administrators",
    operation_id = "resetAdministratorMfa",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Administrator id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 204, description = "Second factor removed (also when there was none)"),
        (status = 403, description = "`permission_denied`, `super_admin_required`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such administrator", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn reset_administrator_mfa(
    State(state): State<AdminState>,
    RequireRecent(who, _): RequireRecent<AdministratorsManage>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    super_admin_only(&who)?;
    let id = parse_admin_id(&id)?;
    let mut tx = begin_locked(&state.db).await?;
    load_target(&mut tx, id).await?;
    let had_factor = admins::reset_mfa(&mut tx, id).await.map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &entry("admin.mfa_reset", &who, id)
            .metadata(serde_json::json!({ "had_factor": had_factor })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    end_sessions(&state, id, RevokeReason::Security).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Sign an administrator out everywhere.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/administrators/{id}/sessions",
    tag = "administrators",
    operation_id = "endAdministratorSessions",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Administrator id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Sessions ended", body = EndedSessions),
        (status = 403, description = "`permission_denied`, `super_admin_required`, `reauth_required` or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such administrator", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn end_administrator_sessions(
    State(state): State<AdminState>,
    RequireRecent(who, _): RequireRecent<AdministratorsManage>,
    Path(id): Path<String>,
) -> Result<Json<EndedSessions>, AppError> {
    super_admin_only(&who)?;
    let id = parse_admin_id(&id)?;
    {
        let mut conn = state.db.acquire().await.map_err(internal)?;
        admins::find_admin(&mut conn, id)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::not_found("administrator"))?;
    }
    let revoked = state
        .sessions
        .revoke_all(id, None, RevokeReason::Admin)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &state.db,
        &entry("admin.sessions_ended", &who, id)
            .metadata(serde_json::json!({ "revoked": revoked })),
    )
    .await
    .map_err(internal)?;
    Ok(Json(EndedSessions { revoked }))
}

/// A permission key that modules declare.
#[derive(Debug, Serialize, ToSchema)]
pub struct PermissionView {
    /// The key, for example `domains.read`.
    #[schema(example = "domains.read")]
    pub key: String,
    /// The module that declares it.
    pub module: String,
    /// That module's display name by language code.
    #[schema(value_type = Object)]
    pub module_title: serde_json::Value,
    /// Declared by the running build. A key that is not stays listed, because
    /// roles may still hold it, but grants nothing that exists.
    pub present: bool,
}

/// The permission catalogue.
#[derive(Debug, Serialize, ToSchema)]
pub struct PermissionList {
    /// By module, then key.
    pub items: Vec<PermissionView>,
}

/// List every permission key the installed modules declare, for the role editor.
#[utoipa::path(
    get,
    path = "/api/v1/admin/permissions",
    tag = "administrators",
    operation_id = "listPermissions",
    security(("session" = [])),
    responses(
        (status = 200, description = "The catalogue, grouped by module", body = PermissionList),
        (status = 403, description = "`permission_denied` (needs admin.roles.read) or `mfa_setup_required`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_permissions(
    State(state): State<AdminState>,
    _who: Require<RolesRead>,
) -> Result<Json<PermissionList>, AppError> {
    let items = builtpanel_database::permissions::list(&state.db)
        .await
        .map_err(AppError::internal)?
        .into_iter()
        .map(|entry| PermissionView {
            key: entry.key,
            module: entry.module,
            module_title: entry.module_title.unwrap_or_else(|| serde_json::json!({})),
            present: entry.present,
        })
        .collect();
    Ok(Json(PermissionList { items }))
}

/// OpenAPI fragment (full paths; merge into the root document).
#[derive(OpenApi)]
#[openapi(
    paths(
        list_permissions, list_administrators, get_administrator, create_administrator, update_administrator,
        delete_administrator, assign_administrator_role, revoke_administrator_role,
        reset_administrator_mfa, end_administrator_sessions
    ),
    components(schemas(
        AdministratorView, AdministratorList, CreateAdministrator, UpdateAdministrator,
        StatusInput, EndedSessions, PermissionView, PermissionList
    )),
    tags((name = "administrators", description = "Manage administrator accounts (super administrators)"))
)]
pub struct AdminApi;

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn create_request_debug_hides_the_password() {
        let req = CreateAdministrator {
            username: "alice".into(),
            email: "a@b.co".into(),
            password: "hunter2-secret".into(),
            role: None,
            require_mfa: None,
        };
        let dbg = format!("{req:?}");
        assert!(dbg.contains("alice") && !dbg.contains("hunter2"), "{dbg}");
    }

    #[test]
    fn roles_of_other_account_kinds_are_not_assignable() {
        assert!(assignable("admin") && assignable("support") && assignable("super_admin"));
        assert!(!assignable("client") && !assignable("reseller"));
    }

    #[test]
    fn malformed_ids_look_like_unknown_ones() {
        assert!(matches!(
            parse_admin_id("nope"),
            Err(AppError::NotFound { .. })
        ));
        assert!(parse_admin_id(&Uuid::now_v7().to_string()).is_ok());
    }

    #[test]
    fn openapi_fragment_lists_every_path() {
        let json = AdminApi::openapi().to_json().expect("json");
        for p in [
            "/api/v1/admin/administrators",
            "/api/v1/admin/administrators/{id}",
            "/api/v1/admin/administrators/{id}/roles/{role}",
            "/api/v1/admin/administrators/{id}/mfa/reset",
            "/api/v1/admin/administrators/{id}/sessions",
        ] {
            assert!(json.contains(p), "{p}");
        }
    }
}
