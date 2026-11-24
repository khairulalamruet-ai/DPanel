//! Resellers (T14.01 - T14.07): accounts that sell hosting to clients of their own.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/admin/resellers` | `admin.resellers.read` / `.manage`, sudo mode |
//! | POST | `/api/v1/admin/resellers/{id}/status` | `admin.resellers.manage`, sudo mode |
//! | DELETE | `/api/v1/admin/resellers/{id}` | `admin.resellers.manage`, sudo mode |
//! | GET, POST | `/api/v1/admin/reseller-packages` | `admin.resellers.read` / `.manage`, sudo mode |
//! | DELETE | `/api/v1/admin/reseller-packages/{id}` | `admin.resellers.manage`, sudo mode |
//! | GET | `/api/v1/reseller/me`, `/me/package` | a reseller |
//! | PUT | `/api/v1/reseller/me/branding` | a reseller |
//! | GET, POST | `/api/v1/reseller/users` | a reseller |
//! | POST | `/api/v1/reseller/users/{id}/status` | a reseller, for its own clients |
//! | DELETE | `/api/v1/reseller/users/{id}` | a reseller, for its own clients |
//! | GET, POST | `/api/v1/reseller/plans` | a reseller |
//! | DELETE | `/api/v1/reseller/plans/{id}` | a reseller |
//!
//! A reseller sees and changes only its own clients and plans: another reseller's
//! (or the administrator's) client answers `404`, as if it did not exist. The package
//! limits are checked in the same transaction that creates the account, with the
//! reseller's row locked, so two requests cannot both take the last free account.
//! A suspended reseller, and its clients, cannot sign in (the database's
//! `effective_account_status`); their sites keep running.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_auth_core::policy::PasswordPolicy;
use builtpanel_database::{
    accounts::{self, DbAccountKind, InsertAccountError, NewAccount},
    plans::{self, Plan},
    rbac::{self, AssignRole},
    resellers::{self, PackageDelete, PackageError, PackageWrite, PlanDelete},
    sqlx::PgConnection,
    users,
};
use builtpanel_domain::{AccountKind, Email, Username};
use builtpanel_http_kit::{AppError, FieldError};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{ResellersManage, ResellersRead},
};
use builtpanel_session::rfc3339;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    CreateUser, UserChange, UsersState, create_user_inner, delete_user_inner, end_sessions,
    internal, suspend_user_inner, unsuspend_user_inner,
};

/// The resellers module's manifest text.
pub const MANIFEST: &str = include_str!("../../../../modules/resellers/module.toml");

const MIB: i64 = 1024 * 1024;
const RESELLER_ROLE: &str = "reseller";
const DEFAULT_COLOR: &str = "#2563eb";

/// The administrator's routes (on the admin application).
pub fn admin_router() -> Router<UsersState> {
    Router::new()
        .route(
            "/api/v1/admin/resellers",
            get(list_resellers).post(create_reseller),
        )
        .route(
            "/api/v1/admin/resellers/{id}/status",
            post(set_reseller_status),
        )
        .route(
            "/api/v1/admin/resellers/{id}",
            axum::routing::delete(delete_reseller),
        )
        .route(
            "/api/v1/admin/reseller-packages",
            get(list_packages).post(create_package),
        )
        .route(
            "/api/v1/admin/reseller-packages/{id}",
            axum::routing::delete(delete_package),
        )
}

/// The reseller's own routes (on the client application).
pub fn portal_router() -> Router<UsersState> {
    Router::new()
        .route("/api/v1/reseller/me", get(me))
        .route("/api/v1/reseller/me/package", get(my_package))
        .route("/api/v1/reseller/me/branding", put(put_branding))
        .route("/api/v1/reseller/users", get(my_users).post(create_my_user))
        .route(
            "/api/v1/reseller/users/{id}/status",
            post(set_my_user_status),
        )
        .route(
            "/api/v1/reseller/users/{id}",
            axum::routing::delete(delete_my_user),
        )
        .route("/api/v1/reseller/plans", get(my_plans).post(create_my_plan))
        .route(
            "/api/v1/reseller/plans/{id}",
            axum::routing::delete(delete_my_plan),
        )
}

// ------------------------------------------------------------------- views

/// A package.
#[derive(Debug, Serialize, ToSchema)]
pub struct PackageView {
    /// Id.
    pub id: Uuid,
    /// Name.
    pub name: String,
    /// Most hosting accounts.
    pub max_accounts: i32,
    /// Disk cap in GiB.
    pub max_disk_gb: i32,
    /// Transfer cap in GiB per month.
    pub max_bandwidth_gb: i32,
    /// Whether the plans may add up to more than the caps.
    pub allow_overselling: bool,
    /// Plans of the administrator the reseller may use.
    pub allowed_plans: Vec<Uuid>,
    /// Creation time.
    pub created_at: String,
}

fn package_view(p: resellers::Package) -> PackageView {
    PackageView {
        id: p.id,
        name: p.name,
        max_accounts: p.max_accounts,
        max_disk_gb: p.max_disk_gb,
        max_bandwidth_gb: p.max_bandwidth_gb,
        allow_overselling: p.allow_overselling,
        allowed_plans: p.allowed_plans,
        created_at: rfc3339(p.created_at),
    }
}

/// A reseller's branding.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BrandingView {
    /// Name shown instead of the panel's.
    pub brand_name: String,
    /// Logo address (https).
    pub logo_url: Option<String>,
    /// Accent colour, `#rrggbb`.
    pub primary_color: String,
    /// Where clients write for support.
    pub support_email: String,
    /// A line shown on the sign-in page.
    pub custom_login_message: Option<String>,
}

/// A reseller.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResellerView {
    /// Id.
    pub id: Uuid,
    /// Login name.
    pub username: String,
    /// Email address.
    pub email: String,
    /// Company name.
    pub company_name: String,
    /// Package id.
    pub package_id: Uuid,
    /// Package name.
    pub package_name: String,
    /// `active` or `suspended`.
    pub status: String,
    /// Hosting accounts of its clients.
    pub accounts_count: i64,
    /// Disk in use, GiB.
    pub disk_used_gb: f64,
    /// Data sent this month, GiB.
    pub bandwidth_used_gb: f64,
    /// Branding.
    pub branding: BrandingView,
    /// Creation time.
    pub created_at: String,
}

#[allow(clippy::cast_precision_loss)]
fn gib(bytes: i64) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0 * 1024.0) * 100.0).round() / 100.0
}

fn text(branding: &Value, key: &str) -> Option<String> {
    branding
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn branding_view(r: &resellers::Reseller) -> BrandingView {
    BrandingView {
        brand_name: text(&r.branding, "brand_name").unwrap_or_else(|| r.company_name.clone()),
        logo_url: text(&r.branding, "logo_url"),
        primary_color: text(&r.branding, "primary_color")
            .unwrap_or_else(|| DEFAULT_COLOR.to_owned()),
        support_email: text(&r.branding, "support_email").unwrap_or_else(|| r.email.clone()),
        custom_login_message: text(&r.branding, "custom_login_message"),
    }
}

fn reseller_view(r: resellers::Reseller) -> ResellerView {
    let branding = branding_view(&r);
    ResellerView {
        id: r.id,
        username: r.username,
        email: r.email,
        company_name: r.company_name,
        package_id: r.package_id,
        package_name: r.package_name,
        status: r.status,
        accounts_count: r.accounts,
        disk_used_gb: gib(r.disk_used_bytes),
        bandwidth_used_gb: gib(r.bandwidth_used_bytes),
        branding,
        created_at: rfc3339(r.created_at),
    }
}

/// A client of a reseller.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClientView {
    /// Account id.
    pub id: Uuid,
    /// The reseller.
    pub reseller_id: Uuid,
    /// Login name.
    pub username: String,
    /// Email address.
    pub email: String,
    /// First domain, empty when it has none yet.
    pub domain: String,
    /// Plan name.
    pub plan_name: String,
    /// Disk in use, MiB.
    pub disk_used_mb: i64,
    /// `active` or `suspended`.
    pub status: String,
    /// Creation time.
    pub created_at: String,
}

fn client_view(reseller_id: Uuid, c: resellers::Client) -> ClientView {
    ClientView {
        id: c.id,
        reseller_id,
        username: c.username,
        email: c.email,
        domain: c.domain,
        plan_name: c.plan_name,
        disk_used_mb: c.disk_used_bytes / MIB,
        status: c.status,
        created_at: rfc3339(c.created_at),
    }
}

/// A plan of a reseller.
#[derive(Debug, Serialize, ToSchema)]
pub struct ResellerPlanView {
    /// Id.
    pub id: Uuid,
    /// The reseller.
    pub reseller_id: Uuid,
    /// Name.
    pub name: String,
    /// Disk, MiB.
    pub disk_mb: i64,
    /// Transfer per month, MiB.
    pub bandwidth_mb: i64,
    /// Most domains.
    pub domains_limit: i64,
    /// Most databases.
    pub databases_limit: i64,
    /// Most mailboxes.
    pub mailboxes_limit: i64,
    /// Creation time.
    pub created_at: String,
}

fn plan_view(reseller_id: Uuid, p: resellers::ResellerPlan) -> ResellerPlanView {
    let limit = |key: &str| p.limits.get(key).and_then(Value::as_i64).unwrap_or(0);
    ResellerPlanView {
        id: p.id,
        reseller_id,
        name: p.name.clone(),
        disk_mb: p.disk_bytes / MIB,
        bandwidth_mb: p.bandwidth_bytes / MIB,
        domains_limit: limit("max_domains"),
        databases_limit: limit("max_databases"),
        mailboxes_limit: limit("max_mailboxes"),
        created_at: rfc3339(p.created_at),
    }
}

fn audit(action: &'static str, who: &Principal, kind: &'static str, target: Uuid) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target(kind, target)
}

// -------------------------------------------------------------- admin: packages

/// A package to create.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePackage {
    /// Name (1-100 characters).
    pub name: String,
    /// Most hosting accounts.
    pub max_accounts: i32,
    /// Disk cap in GiB.
    pub max_disk_gb: i32,
    /// Transfer cap in GiB per month.
    pub max_bandwidth_gb: i32,
    /// Whether the plans may add up to more than the caps.
    #[serde(default)]
    pub allow_overselling: bool,
    /// Plans of the administrator the reseller may use.
    #[serde(default)]
    pub allowed_plans: Vec<Uuid>,
}

/// The packages.
#[utoipa::path(
    get, path = "/api/v1/admin/reseller-packages", tag = "resellers",
    operation_id = "listResellerPackages", security(("session" = [])),
    responses((status = 200, description = "The packages", body = Vec<PackageView>))
)]
pub async fn list_packages(
    State(state): State<UsersState>,
    _who: Require<ResellersRead>,
) -> Result<Json<Vec<PackageView>>, AppError> {
    let rows = resellers::packages(&state.db).await.map_err(internal)?;
    Ok(Json(rows.into_iter().map(package_view).collect()))
}

/// Create a package.
#[utoipa::path(
    post, path = "/api/v1/admin/reseller-packages", tag = "resellers",
    operation_id = "createResellerPackage", security(("session" = [])),
    request_body = CreatePackage,
    responses(
        (status = 201, description = "Created", body = PackageView),
        (status = 409, description = "`package_name_taken`"),
        (status = 422, description = "Invalid input")
    )
)]
pub async fn create_package(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<ResellersManage>,
    Json(req): Json<CreatePackage>,
) -> Result<(StatusCode, Json<PackageView>), AppError> {
    let mut errors = Vec::new();
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        errors.push(FieldError::new(
            "name",
            "invalid_name",
            "Use 1 to 100 characters.",
        ));
    }
    for (field, v, max) in [
        ("max_accounts", req.max_accounts, 1_000_000),
        ("max_disk_gb", req.max_disk_gb, 100_000_000),
        ("max_bandwidth_gb", req.max_bandwidth_gb, 100_000_000),
    ] {
        if !(0..=max).contains(&v) {
            errors.push(FieldError::new(field, "out_of_range", "Out of range."));
        }
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    let id = resellers::insert_package(
        &mut tx,
        &PackageWrite {
            name,
            max_accounts: req.max_accounts,
            max_disk_gb: req.max_disk_gb,
            max_bandwidth_gb: req.max_bandwidth_gb,
            allow_overselling: req.allow_overselling,
            allowed_plans: &req.allowed_plans,
        },
    )
    .await
    .map_err(|e| match e {
        PackageError::NameTaken => {
            AppError::conflict("package_name_taken", "A package with this name exists.")
        }
        PackageError::UnknownPlan => AppError::validation(vec![FieldError::new(
            "allowed_plans",
            "unknown_plan",
            "Every allowed plan must be one of the administrator's plans.",
        )]),
        PackageError::Database(db) => internal(db),
    })?;
    builtpanel_audit::insert(
        &mut *tx,
        &audit(
            "admin.reseller_package_created",
            &who,
            "reseller_package",
            id,
        ),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let p = resellers::package(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("package"))?;
    Ok((StatusCode::CREATED, Json(package_view(p))))
}

/// Delete a package no reseller uses.
#[utoipa::path(
    delete, path = "/api/v1/admin/reseller-packages/{id}", tag = "resellers",
    operation_id = "deleteResellerPackage", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Package id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such package"),
        (status = 409, description = "`package_in_use`")
    )
)]
pub async fn delete_package(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<ResellersManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let mut tx = state.db.begin().await.map_err(internal)?;
    match resellers::delete_package(&mut tx, id)
        .await
        .map_err(internal)?
    {
        PackageDelete::Unknown => return Err(AppError::not_found("package")),
        PackageDelete::InUse => {
            return Err(AppError::conflict(
                "package_in_use",
                "Resellers use this package; move them to another one first.",
            ));
        }
        PackageDelete::Deleted => {}
    }
    builtpanel_audit::insert(
        &mut *tx,
        &audit(
            "admin.reseller_package_deleted",
            &who,
            "reseller_package",
            id,
        ),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------- admin: resellers

/// A reseller to create.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateReseller {
    /// Login name.
    pub username: String,
    /// Email address.
    pub email: String,
    /// Company name.
    pub company_name: String,
    /// Initial password; must satisfy the password policy.
    #[schema(format = Password)]
    pub password: String,
    /// The package.
    pub package_id: Uuid,
}

/// The resellers.
#[utoipa::path(
    get, path = "/api/v1/admin/resellers", tag = "resellers",
    operation_id = "listResellers", security(("session" = [])),
    responses((status = 200, description = "The resellers, newest first", body = Vec<ResellerView>))
)]
pub async fn list_resellers(
    State(state): State<UsersState>,
    _who: Require<ResellersRead>,
) -> Result<Json<Vec<ResellerView>>, AppError> {
    let rows = resellers::list(&state.db).await.map_err(internal)?;
    Ok(Json(rows.into_iter().map(reseller_view).collect()))
}

/// Create a reseller: the account, its role and its standing, in one transaction.
#[utoipa::path(
    post, path = "/api/v1/admin/resellers", tag = "resellers",
    operation_id = "createReseller", security(("session" = [])),
    request_body = CreateReseller,
    responses(
        (status = 201, description = "Created", body = ResellerView),
        (status = 409, description = "`username_taken` or `email_taken`"),
        (status = 422, description = "Invalid input or an unknown package")
    )
)]
pub async fn create_reseller(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<ResellersManage>,
    Json(req): Json<CreateReseller>,
) -> Result<(StatusCode, Json<ResellerView>), AppError> {
    let mut errors = Vec::new();
    let username = Username::parse(&req.username)
        .map_err(|e| errors.push(FieldError::new("username", e.code(), e.to_string())))
        .ok();
    let email = Email::parse(&req.email)
        .map_err(|e| errors.push(FieldError::new("email", e.code(), e.to_string())))
        .ok();
    let company = req.company_name.trim();
    if company.is_empty() || company.chars().count() > 100 {
        errors.push(FieldError::new(
            "company_name",
            "invalid_company_name",
            "Use 1 to 100 characters.",
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
    if resellers::package(&state.db, req.package_id)
        .await
        .map_err(internal)?
        .is_none()
    {
        return Err(AppError::validation(vec![FieldError::new(
            "package_id",
            "unknown_package",
            "There is no such package.",
        )]));
    }
    let hash = state.hasher.hash_async(password).await.map_err(internal)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let id = accounts::insert_account_in(
        &mut tx,
        &NewAccount {
            kind: AccountKind::Reseller,
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
    match rbac::assign_role_in(&mut tx, id, RESELLER_ROLE, Some(who.account_id()))
        .await
        .map_err(internal)?
    {
        AssignRole::Assigned | AssignRole::AlreadyAssigned => {}
        AssignRole::UnknownRole => return Err(internal(std::io::Error::other("no reseller role"))),
    }
    resellers::insert(&mut tx, id, company, req.package_id, &json!({}))
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &audit("admin.reseller_created", &who, "reseller", id).metadata(json!({
            "username": username.as_str(),
            "package_id": req.package_id,
        })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let r = resellers::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("reseller"))?;
    Ok((StatusCode::CREATED, Json(reseller_view(r))))
}

/// A new status for a reseller.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetStatus {
    /// `active` or `suspended`.
    pub status: String,
}

fn parse_status(s: &str) -> Result<&'static str, AppError> {
    match s {
        "active" => Ok("active"),
        "suspended" => Ok("suspended"),
        _ => Err(AppError::validation(vec![FieldError::new(
            "status",
            "invalid_status",
            "Expected active or suspended.",
        )])),
    }
}

/// Suspend or resume a reseller. While it is suspended neither it nor its clients can
/// sign in; the clients' sites keep running.
#[utoipa::path(
    post, path = "/api/v1/admin/resellers/{id}/status", tag = "resellers",
    operation_id = "setResellerStatus", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Reseller id")),
    request_body = SetStatus,
    responses((status = 200, description = "Changed", body = ResellerView), (status = 404, description = "No such reseller"))
)]
pub async fn set_reseller_status(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<ResellersManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SetStatus>,
) -> Result<Json<ResellerView>, AppError> {
    let status = parse_status(&req.status)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !resellers::set_status(&mut tx, id, status)
        .await
        .map_err(internal)?
    {
        return Err(AppError::not_found("reseller"));
    }
    builtpanel_audit::insert(
        &mut *tx,
        &audit("admin.reseller_status_changed", &who, "reseller", id)
            .metadata(json!({ "status": status })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    if status == "suspended" {
        end_sessions(&state, id).await;
        for client in resellers::client_ids(&state.db, id)
            .await
            .map_err(internal)?
        {
            end_sessions(&state, client).await;
        }
    }
    let r = resellers::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("reseller"))?;
    Ok(Json(reseller_view(r)))
}

/// Delete a reseller that has no hosting accounts (any it had must be deleted and purged
/// first). Its plans go; the login is disabled.
#[utoipa::path(
    delete, path = "/api/v1/admin/resellers/{id}", tag = "resellers",
    operation_id = "deleteReseller", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Reseller id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such reseller"),
        (status = 409, description = "`reseller_has_accounts`")
    )
)]
pub async fn delete_reseller(
    State(state): State<UsersState>,
    RequireRecent(who, _): RequireRecent<ResellersManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if resellers::get(&state.db, id)
        .await
        .map_err(internal)?
        .is_none()
    {
        return Err(AppError::not_found("reseller"));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    if !resellers::delete(&mut tx, id).await.map_err(internal)? {
        return Err(AppError::conflict(
            "reseller_has_accounts",
            "This reseller still has hosting accounts. Delete them first.",
        ));
    }
    users::set_account_status(&mut tx, id, "disabled")
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &audit("admin.reseller_deleted", &who, "reseller", id),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    end_sessions(&state, id).await;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------- reseller's own

/// The reseller the caller is, when it is one and not suspended.
async fn acting(state: &UsersState, who: &Principal) -> Result<resellers::Reseller, AppError> {
    if who.kind != DbAccountKind::Reseller {
        return Err(AppError::forbidden(
            "not_a_reseller",
            "Only a reseller can do this.",
        ));
    }
    let r = resellers::get(&state.db, who.account_id())
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::forbidden("not_a_reseller", "Only a reseller can do this."))?;
    if r.status != "active" {
        return Err(AppError::forbidden(
            "reseller_suspended",
            "This reseller account is suspended.",
        ));
    }
    Ok(r)
}

/// The caller's own reseller account.
#[utoipa::path(
    get, path = "/api/v1/reseller/me", tag = "resellers",
    operation_id = "getResellerMe", security(("session" = [])),
    responses((status = 200, description = "The reseller", body = ResellerView), (status = 403, description = "`not_a_reseller`"))
)]
pub async fn me(
    State(state): State<UsersState>,
    who: Principal,
) -> Result<Json<ResellerView>, AppError> {
    Ok(Json(reseller_view(acting(&state, &who).await?)))
}

/// The caller's package.
#[utoipa::path(
    get, path = "/api/v1/reseller/me/package", tag = "resellers",
    operation_id = "getResellerPackage", security(("session" = [])),
    responses((status = 200, description = "The package", body = PackageView), (status = 403, description = "`not_a_reseller`"))
)]
pub async fn my_package(
    State(state): State<UsersState>,
    who: Principal,
) -> Result<Json<PackageView>, AppError> {
    let r = acting(&state, &who).await?;
    let p = resellers::package(&state.db, r.package_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("package"))?;
    Ok(Json(package_view(p)))
}

/// New branding; fields left out stay as they are.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PutBranding {
    /// Name shown instead of the panel's (1-100 characters).
    pub brand_name: Option<String>,
    /// Logo address; https only. An empty string removes it.
    pub logo_url: Option<String>,
    /// Accent colour, `#rrggbb`.
    pub primary_color: Option<String>,
    /// Support email address.
    pub support_email: Option<String>,
    /// A line for the sign-in page (up to 300 characters). An empty string removes it.
    pub custom_login_message: Option<String>,
}

fn is_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Change the branding.
#[utoipa::path(
    put, path = "/api/v1/reseller/me/branding", tag = "resellers",
    operation_id = "putResellerBranding", security(("session" = [])),
    request_body = PutBranding,
    responses((status = 200, description = "The branding now", body = BrandingView), (status = 422, description = "Invalid input"))
)]
pub async fn put_branding(
    State(state): State<UsersState>,
    who: Principal,
    Json(req): Json<PutBranding>,
) -> Result<Json<BrandingView>, AppError> {
    let r = acting(&state, &who).await?;
    let mut branding = r.branding.clone();
    let obj = branding
        .as_object_mut()
        .ok_or_else(|| internal(std::io::Error::other("branding is not an object")))?;
    let mut errors = Vec::new();
    if let Some(v) = &req.brand_name {
        let v = v.trim();
        if v.is_empty() || v.chars().count() > 100 {
            errors.push(FieldError::new(
                "brand_name",
                "invalid_brand_name",
                "Use 1 to 100 characters.",
            ));
        }
        obj.insert("brand_name".into(), json!(v));
    }
    if let Some(v) = &req.logo_url {
        if v.is_empty() {
            obj.remove("logo_url");
        } else if v.len() > 500 || !v.starts_with("https://") || v.chars().any(char::is_whitespace)
        {
            errors.push(FieldError::new(
                "logo_url",
                "invalid_logo_url",
                "Use an https address of up to 500 characters.",
            ));
        } else {
            obj.insert("logo_url".into(), json!(v));
        }
    }
    if let Some(v) = &req.primary_color {
        if is_color(v) {
            obj.insert("primary_color".into(), json!(v.to_ascii_lowercase()));
        } else {
            errors.push(FieldError::new(
                "primary_color",
                "invalid_color",
                "Use a colour like #2563eb.",
            ));
        }
    }
    if let Some(v) = &req.support_email {
        match Email::parse(v) {
            Ok(e) => {
                obj.insert("support_email".into(), json!(e.as_str()));
            }
            Err(e) => errors.push(FieldError::new("support_email", e.code(), e.to_string())),
        }
    }
    if let Some(v) = &req.custom_login_message {
        if v.is_empty() {
            obj.remove("custom_login_message");
        } else if v.chars().count() > 300 || v.chars().any(char::is_control) {
            errors.push(FieldError::new(
                "custom_login_message",
                "invalid_message",
                "Use up to 300 characters on one line.",
            ));
        } else {
            obj.insert("custom_login_message".into(), json!(v));
        }
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    resellers::set_branding(&mut tx, r.id, &branding)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &audit("reseller.branding_changed", &who, "reseller", r.id),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let updated = resellers::Reseller { branding, ..r };
    Ok(Json(branding_view(&updated)))
}

/// The caller's clients.
#[utoipa::path(
    get, path = "/api/v1/reseller/users", tag = "resellers",
    operation_id = "listResellerUsers", security(("session" = [])),
    responses((status = 200, description = "The clients, newest first", body = Vec<ClientView>))
)]
pub async fn my_users(
    State(state): State<UsersState>,
    who: Principal,
) -> Result<Json<Vec<ClientView>>, AppError> {
    let r = acting(&state, &who).await?;
    let rows = resellers::clients(&state.db, r.id)
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.into_iter().map(|c| client_view(r.id, c)).collect(),
    ))
}

/// A client to create.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateClient {
    /// Login name.
    pub username: String,
    /// Email address.
    pub email: String,
    /// Initial password; must satisfy the password policy.
    #[schema(format = Password)]
    pub password: String,
    /// The plan, by id.
    pub plan_id: Option<Uuid>,
    /// The plan, by the name the reseller gave it (or an allowed plan's name).
    pub plan_name: Option<String>,
    /// The client's first domain. It is not set up here: the client adds it from its own
    /// panel.
    pub domain: Option<String>,
}

async fn plan_for(
    state: &UsersState,
    reseller: &resellers::Reseller,
    by_id: Option<Uuid>,
    by_name: Option<&str>,
) -> Result<Plan, AppError> {
    let not_found = || {
        AppError::validation(vec![FieldError::new(
            "plan_name",
            "unknown_plan",
            "There is no such plan among yours.",
        )])
    };
    let own = resellers::plans_of(&state.db, reseller.id)
        .await
        .map_err(internal)?;
    let package = resellers::package(&state.db, reseller.package_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("package"))?;
    let id = match (by_id, by_name) {
        (Some(id), _) => id,
        (None, Some(name)) => {
            if let Some(p) = own.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
                p.id
            } else {
                let mut found = None;
                for id in &package.allowed_plans {
                    if let Some(p) = plans::get(&state.db, *id).await.map_err(internal)?
                        && p.name == name
                    {
                        found = Some(p.id);
                    }
                }
                found.ok_or_else(not_found)?
            }
        }
        (None, None) => return Err(not_found()),
    };
    plans::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(not_found)
}

/// Create a client of the caller.
#[utoipa::path(
    post, path = "/api/v1/reseller/users", tag = "resellers",
    operation_id = "createResellerUser", security(("session" = [])),
    request_body = CreateClient,
    responses(
        (status = 201, description = "Created", body = ClientView),
        (status = 409, description = "`reseller_limit` (the package is used up), `username_taken`, `email_taken`..."),
        (status = 422, description = "Invalid input or a plan that is not the caller's")
    )
)]
pub async fn create_my_user(
    State(state): State<UsersState>,
    who: Principal,
    Json(req): Json<CreateClient>,
) -> Result<(StatusCode, Json<ClientView>), AppError> {
    let r = acting(&state, &who).await?;
    let plan = plan_for(&state, &r, req.plan_id, req.plan_name.as_deref()).await?;
    let (status, Json(change)) = create_user_inner(
        &state,
        &who,
        CreateUser {
            username: req.username,
            email: req.email,
            password: req.password,
            plan_id: Some(plan.id),
            node_id: None,
            system_username: None,
            locale: None,
        },
        Some(r.id),
    )
    .await?;
    let client = resellers::clients(&state.db, r.id)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|c| c.id == change.user.id)
        .ok_or_else(|| AppError::not_found("user"))?;
    Ok((status, Json(client_view(r.id, client))))
}

/// A new status for a client.
async fn own_client(state: &UsersState, reseller: Uuid, id: Uuid) -> Result<(), AppError> {
    if resellers::owns_client(&state.db, reseller, id)
        .await
        .map_err(internal)?
    {
        Ok(())
    } else {
        Err(AppError::not_found("user"))
    }
}

/// Suspend or resume one of the caller's clients.
#[utoipa::path(
    post, path = "/api/v1/reseller/users/{id}/status", tag = "resellers",
    operation_id = "setResellerUserStatus", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Client id")),
    request_body = SetStatus,
    responses((status = 200, description = "Changed", body = UserChange), (status = 404, description = "Not one of the caller's clients"))
)]
pub async fn set_my_user_status(
    State(state): State<UsersState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<SetStatus>,
) -> Result<Json<UserChange>, AppError> {
    let r = acting(&state, &who).await?;
    let status = parse_status(&req.status)?;
    own_client(&state, r.id, id).await?;
    if status == "suspended" {
        suspend_user_inner(&state, &who, id, true).await
    } else {
        unsuspend_user_inner(&state, &who, id, true).await
    }
}

/// Delete one of the caller's clients (its data is kept for a while, as for any user).
#[utoipa::path(
    delete, path = "/api/v1/reseller/users/{id}", tag = "resellers",
    operation_id = "deleteResellerUser", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Client id")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "Not one of the caller's clients"))
)]
pub async fn delete_my_user(
    State(state): State<UsersState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let r = acting(&state, &who).await?;
    own_client(&state, r.id, id).await?;
    let Json(_) = delete_user_inner(&state, &who, id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The caller's plans.
#[utoipa::path(
    get, path = "/api/v1/reseller/plans", tag = "resellers",
    operation_id = "listResellerPlans", security(("session" = [])),
    responses((status = 200, description = "The plans", body = Vec<ResellerPlanView>))
)]
pub async fn my_plans(
    State(state): State<UsersState>,
    who: Principal,
) -> Result<Json<Vec<ResellerPlanView>>, AppError> {
    let r = acting(&state, &who).await?;
    let rows = resellers::plans_of(&state.db, r.id)
        .await
        .map_err(internal)?;
    Ok(Json(rows.into_iter().map(|p| plan_view(r.id, p)).collect()))
}

/// A plan to create.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePlan {
    /// Name (1-100 characters).
    pub name: String,
    /// Disk in MiB.
    pub disk_mb: i64,
    /// Transfer per month in MiB.
    pub bandwidth_mb: i64,
    /// Most domains.
    pub domains_limit: i64,
    /// Most databases.
    pub databases_limit: i64,
    /// Most mailboxes.
    pub mailboxes_limit: i64,
}

/// Create a plan of the caller.
#[utoipa::path(
    post, path = "/api/v1/reseller/plans", tag = "resellers",
    operation_id = "createResellerPlan", security(("session" = [])),
    request_body = CreatePlan,
    responses((status = 201, description = "Created", body = ResellerPlanView), (status = 409, description = "`plan_name_taken`"), (status = 422, description = "Invalid input"))
)]
pub async fn create_my_plan(
    State(state): State<UsersState>,
    who: Principal,
    Json(req): Json<CreatePlan>,
) -> Result<(StatusCode, Json<ResellerPlanView>), AppError> {
    let r = acting(&state, &who).await?;
    let mut errors = Vec::new();
    let name = req.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        errors.push(FieldError::new(
            "name",
            "invalid_name",
            "Use 1 to 100 characters.",
        ));
    }
    for (field, v, min, max) in [
        ("disk_mb", req.disk_mb, 1, 100_000_000),
        ("bandwidth_mb", req.bandwidth_mb, 0, 1_000_000_000),
        ("domains_limit", req.domains_limit, 0, 1_000_000),
        ("databases_limit", req.databases_limit, 0, 1_000_000),
        ("mailboxes_limit", req.mailboxes_limit, 0, 1_000_000),
    ] {
        if !(min..=max).contains(&v) {
            errors.push(FieldError::new(field, "out_of_range", "Out of range."));
        }
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    // The same name twice would confuse "pick a plan by name".
    let taken = resellers::plans_of(&state.db, r.id)
        .await
        .map_err(internal)?
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case(name));
    if taken {
        return Err(AppError::conflict(
            "plan_name_taken",
            "You already have a plan with this name.",
        ));
    }
    let id = resellers::insert_plan(
        &mut tx,
        r.id,
        name,
        req.disk_mb * MIB,
        req.bandwidth_mb * MIB,
        &json!({
            "max_domains": req.domains_limit,
            "max_databases": req.databases_limit,
            "max_mailboxes": req.mailboxes_limit,
        }),
    )
    .await
    .map_err(internal)?;
    builtpanel_audit::insert(&mut *tx, &audit("reseller.plan_created", &who, "plan", id))
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let p = resellers::plans_of(&state.db, r.id)
        .await
        .map_err(internal)?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| AppError::not_found("plan"))?;
    Ok((StatusCode::CREATED, Json(plan_view(r.id, p))))
}

/// Delete a plan of the caller that no account uses.
#[utoipa::path(
    delete, path = "/api/v1/reseller/plans/{id}", tag = "resellers",
    operation_id = "deleteResellerPlan", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Plan id")),
    responses((status = 204, description = "Deleted"), (status = 404, description = "Not one of the caller's plans"), (status = 409, description = "`plan_in_use`"))
)]
pub async fn delete_my_plan(
    State(state): State<UsersState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let r = acting(&state, &who).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    match resellers::delete_plan(&mut tx, r.id, id)
        .await
        .map_err(internal)?
    {
        PlanDelete::Unknown => return Err(AppError::not_found("plan")),
        PlanDelete::InUse => {
            return Err(AppError::conflict(
                "plan_in_use",
                "Accounts are on this plan; move them first.",
            ));
        }
        PlanDelete::Deleted => {}
    }
    builtpanel_audit::insert(&mut *tx, &audit("reseller.plan_deleted", &who, "plan", id))
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// -------------------------------------------------------------- limits

/// Checks, with the reseller's row locked, that one more account on `plan` fits the
/// package. Called inside the transaction that creates the account.
///
/// # Errors
/// 403 `reseller_suspended`; 409 `reseller_limit`; 422 when the plan is not the
/// reseller's own or one of the package's allowed plans.
pub(crate) async fn check_room(
    conn: &mut PgConnection,
    reseller_id: Uuid,
    plan: &Plan,
) -> Result<(), AppError> {
    let standing = resellers::standing_locked(conn, reseller_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::forbidden("not_a_reseller", "Only a reseller can do this."))?;
    if standing.status != "active" {
        return Err(AppError::forbidden(
            "reseller_suspended",
            "This reseller account is suspended.",
        ));
    }
    let facts = resellers::plan_facts(conn, plan.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("plan"))?;
    let allowed = match facts.reseller_id {
        Some(owner) => owner == reseller_id,
        None => standing.allowed_plans.contains(&plan.id),
    };
    if !allowed {
        return Err(AppError::validation(vec![FieldError::new(
            "plan_name",
            "plan_not_allowed",
            "This plan is not available to you.",
        )]));
    }
    let held = resellers::allocated(conn, reseller_id)
        .await
        .map_err(internal)?;
    let limit = |what: &str, used: i64, cap: i64| {
        AppError::conflict(
            "reseller_limit",
            format!("Your package allows {cap} {what}; {used} are in use."),
        )
    };
    if held.accounts + 1 > i64::from(standing.max_accounts) {
        return Err(limit(
            "hosting accounts",
            held.accounts,
            i64::from(standing.max_accounts),
        ));
    }
    if !standing.allow_overselling {
        // A plan without a cap can never fit under a cap.
        let fits = |held: Option<i64>, add: Option<i64>, cap: i64| match (held, add) {
            (Some(h), Some(a)) => h.saturating_add(a) <= cap,
            _ => false,
        };
        if !fits(held.disk_bytes, plan.disk_bytes, standing.max_disk_bytes) {
            return Err(AppError::conflict(
                "reseller_limit",
                "The disk space of your accounts would pass your package's limit.",
            ));
        }
        if !fits(
            held.bandwidth_bytes,
            plan.bandwidth_bytes,
            standing.max_bandwidth_bytes,
        ) {
            return Err(AppError::conflict(
                "reseller_limit",
                "The monthly transfer of your accounts would pass your package's limit.",
            ));
        }
    }
    Ok(())
}
