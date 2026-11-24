//! The administrators' API (T17.05): customers, products, plans and licenses.
//!
//! Everything below `/api/v1/admin` needs a bearer token from `POST /api/v1/admin/login`
//! (only its SHA-256 is kept). Viewers can read; admins can change. Every change is audited.
//! A license's key is shown once, when it is made: only its hash is stored.

use std::io::Read as _;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
};
use builtpanel_http_kit::{AppError, FieldError};
use builtpanel_license::FileSigningKey;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{Admin, LaState, audit, sha256};

/// The routes.
pub fn router() -> Router<LaState> {
    Router::new()
        .route("/api/v1/admin/login", post(login))
        .route("/api/v1/admin/logout", post(logout))
        .route(
            "/api/v1/admin/customers",
            get(list_customers).post(add_customer),
        )
        .route("/api/v1/admin/customers/{id}", get(get_customer))
        .route(
            "/api/v1/admin/products",
            get(list_products).post(add_product),
        )
        .route("/api/v1/admin/plans", get(list_plans).post(add_plan))
        .route(
            "/api/v1/admin/licenses",
            get(list_licenses).post(add_license),
        )
        .route("/api/v1/admin/licenses/{id}", get(get_license))
        .route("/api/v1/admin/licenses/{id}/binding", put(set_binding))
        .route("/api/v1/admin/licenses/{id}/suspend", post(suspend))
        .route("/api/v1/admin/licenses/{id}/resume", post(resume))
        .route("/api/v1/admin/licenses/{id}/revoke", post(revoke))
        .route("/api/v1/admin/licenses/{id}/renew", post(renew))
        .route("/api/v1/admin/activations", get(list_activations))
        .route(
            "/api/v1/admin/activations/{id}/deactivate",
            post(deactivate),
        )
        .route("/api/v1/admin/keys", get(list_keys))
        .route("/api/v1/admin/keys/rotate", post(rotate_keys))
        .route("/api/v1/admin/audit", get(list_audit))
}

fn bad(field: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(field, "invalid", message)])
}

fn db(e: sqlx::Error) -> AppError {
    AppError::internal(e)
}

// ---------------------------------------------------------------- sign in

/// Credentials.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Login {
    /// Email address.
    pub email: String,
    /// Password.
    pub password: String,
}

/// A sign-in.
#[derive(Serialize)]
pub struct Session {
    /// The bearer token (shown once).
    pub token: String,
    /// When it stops working (Unix seconds).
    pub expires_at: i64,
    /// `admin` or `viewer`.
    pub role: String,
}

fn random_bytes(n: usize) -> Result<Vec<u8>, AppError> {
    let mut bytes = vec![0u8; n];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(AppError::internal)?;
    Ok(bytes)
}

async fn login(
    State(state): State<LaState>,
    Json(req): Json<Login>,
) -> Result<Json<Session>, AppError> {
    let wrong = || {
        AppError::authentication_failed("invalid_credentials", "The email or password is wrong.")
    };
    let row: Option<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, password_hash, role FROM admin_users WHERE lower(email) = lower($1)",
    )
    .bind(&req.email)
    .fetch_optional(&state.db)
    .await
    .map_err(db)?;
    let password = SecretString::from(req.password);
    let Some((id, hash, role)) = row else {
        // The same work whether or not the address exists.
        let _ = state.hasher.verify_dummy(&password);
        return Err(wrong());
    };
    let ok = state
        .hasher
        .verify(&password, &hash)
        .map_err(AppError::internal)?
        .matches;
    if !ok {
        return Err(wrong());
    }
    let token: String = random_bytes(32)?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let ttl = i64::try_from(state.settings.session_ttl.as_secs()).unwrap_or(3600);
    let expires_at: i64 = sqlx::query_scalar(
        "INSERT INTO admin_sessions (token_hash, admin_id, expires_at)
         VALUES ($1, $2, now() + make_interval(secs => $3::double precision))
         RETURNING extract(epoch FROM expires_at)::bigint",
    )
    .bind(sha256(&token))
    .bind(id)
    .bind(ttl)
    .fetch_one(&state.db)
    .await
    .map_err(db)?;
    sqlx::query("DELETE FROM admin_sessions WHERE expires_at < now()")
        .execute(&state.db)
        .await
        .map_err(db)?;
    audit(
        &state.db,
        &req.email,
        "admin.login",
        None,
        serde_json::json!({}),
    )
    .await?;
    Ok(Json(Session {
        token,
        expires_at,
        role,
    }))
}

async fn logout(
    State(state): State<LaState>,
    headers: axum::http::HeaderMap,
    _admin: Admin,
) -> Result<StatusCode, AppError> {
    if let Some(token) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        sqlx::query("DELETE FROM admin_sessions WHERE token_hash = $1")
            .bind(sha256(token))
            .execute(&state.db)
            .await
            .map_err(db)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- customers

/// A customer.
#[derive(Serialize, FromRow)]
pub struct Customer {
    /// Id.
    pub id: Uuid,
    /// Name.
    pub name: String,
    /// Email.
    pub email: String,
    /// Notes.
    pub notes: Option<String>,
    /// Created (Unix seconds).
    pub created_at: i64,
}

macro_rules! customer {
    ($($tail:literal),*) => {
        concat!("SELECT id, name, email, notes, extract(epoch FROM created_at)::bigint AS created_at FROM customers"$(, $tail)*)
    };
}

/// A customer to add.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewCustomer {
    /// Name.
    pub name: String,
    /// Email.
    pub email: String,
    /// Notes.
    #[serde(default)]
    pub notes: Option<String>,
}

async fn list_customers(
    State(state): State<LaState>,
    _a: Admin,
) -> Result<Json<Vec<Customer>>, AppError> {
    sqlx::query_as(customer!(" ORDER BY created_at DESC LIMIT 500"))
        .fetch_all(&state.db)
        .await
        .map(Json)
        .map_err(db)
}

async fn get_customer(
    State(state): State<LaState>,
    _a: Admin,
    Path(id): Path<Uuid>,
) -> Result<Json<Customer>, AppError> {
    sqlx::query_as(customer!(" WHERE id = $1"))
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(db)?
        .map(Json)
        .ok_or_else(|| AppError::not_found("customer"))
}

async fn add_customer(
    State(state): State<LaState>,
    admin: Admin,
    Json(req): Json<NewCustomer>,
) -> Result<(StatusCode, Json<Customer>), AppError> {
    admin.may_change()?;
    let name = req.name.trim();
    let email = req.email.trim();
    if name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
        return Err(bad("name", "1 to 200 characters."));
    }
    if !email.contains('@')
        || email.len() > 254
        || email.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(bad("email", "An email address."));
    }
    if req.notes.as_deref().is_some_and(|n| n.len() > 2000) {
        return Err(bad("notes", "At most 2000 characters."));
    }
    let made: Result<Customer, sqlx::Error> = sqlx::query_as(
        "INSERT INTO customers (name, email, notes) VALUES ($1, $2, $3)
         RETURNING id, name, email, notes, extract(epoch FROM created_at)::bigint AS created_at",
    )
    .bind(name)
    .bind(email)
    .bind(req.notes.as_deref())
    .fetch_one(&state.db)
    .await;
    let made = made.map_err(|e| {
        if e.as_database_error().and_then(|d| d.constraint()) == Some("customers_email_idx") {
            AppError::conflict("email_taken", "A customer with this email exists already.")
        } else {
            db(e)
        }
    })?;
    audit(
        &state.db,
        &admin.email,
        "customer.created",
        Some(&made.id.to_string()),
        serde_json::json!({}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(made)))
}

// ---------------------------------------------------------------- products and plans

/// A product.
#[derive(Serialize, FromRow)]
pub struct Product {
    /// Id.
    pub id: Uuid,
    /// Slug.
    pub slug: String,
    /// Name.
    pub name: String,
}

/// A product to add.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewProduct {
    /// Slug (`builtpanel`).
    pub slug: String,
    /// Name.
    pub name: String,
}

async fn list_products(
    State(state): State<LaState>,
    _a: Admin,
) -> Result<Json<Vec<Product>>, AppError> {
    sqlx::query_as("SELECT id, slug, name FROM products ORDER BY slug")
        .fetch_all(&state.db)
        .await
        .map(Json)
        .map_err(db)
}

async fn add_product(
    State(state): State<LaState>,
    admin: Admin,
    Json(req): Json<NewProduct>,
) -> Result<(StatusCode, Json<Product>), AppError> {
    admin.may_change()?;
    if req.name.trim().is_empty() || req.name.chars().count() > 100 {
        return Err(bad("name", "1 to 100 characters."));
    }
    let made: Result<Product, sqlx::Error> = sqlx::query_as(
        "INSERT INTO products (slug, name) VALUES ($1, $2) RETURNING id, slug, name",
    )
    .bind(&req.slug)
    .bind(req.name.trim())
    .fetch_one(&state.db)
    .await;
    let made = made.map_err(|e| {
        match e
            .as_database_error()
            .map(|d| d.code().map(|c| c.to_string()))
        {
            Some(Some(code)) if code == "23505" => {
                AppError::conflict("slug_taken", "This slug is taken.")
            }
            Some(Some(code)) if code == "23514" => {
                bad("slug", "Lower-case letters, digits and - (2 to 41).")
            }
            _ => db(e),
        }
    })?;
    audit(
        &state.db,
        &admin.email,
        "product.created",
        Some(&made.id.to_string()),
        serde_json::json!({}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(made)))
}

/// A plan.
#[derive(Serialize, FromRow)]
pub struct Plan {
    /// Id.
    pub id: Uuid,
    /// The product.
    pub product_id: Uuid,
    /// Name.
    pub name: String,
    /// What it switches on.
    pub features: Vec<String>,
    /// Numeric limits (-1: none).
    pub limits: serde_json::Value,
    /// Installations at once.
    pub max_activations: i32,
    /// How long a new license lasts (days).
    pub valid_days: Option<i32>,
}

/// A plan to add.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPlan {
    /// The product.
    pub product_id: Uuid,
    /// Name.
    pub name: String,
    /// What it switches on.
    #[serde(default)]
    pub features: Vec<String>,
    /// Numeric limits.
    #[serde(default)]
    pub limits: std::collections::BTreeMap<String, i64>,
    /// Installations at once.
    #[serde(default = "one")]
    pub max_activations: i32,
    /// How long a new license lasts (days).
    #[serde(default)]
    pub valid_days: Option<i32>,
}

fn one() -> i32 {
    1
}

/// Which plans.
#[derive(Deserialize)]
pub struct PlanQuery {
    /// Only this product's.
    pub product_id: Option<Uuid>,
}

async fn list_plans(
    State(state): State<LaState>,
    _a: Admin,
    Query(q): Query<PlanQuery>,
) -> Result<Json<Vec<Plan>>, AppError> {
    sqlx::query_as(
        "SELECT id, product_id, name, features, limits, max_activations, valid_days FROM plans
         WHERE ($1::uuid IS NULL OR product_id = $1) ORDER BY name",
    )
    .bind(q.product_id)
    .fetch_all(&state.db)
    .await
    .map(Json)
    .map_err(db)
}

async fn add_plan(
    State(state): State<LaState>,
    admin: Admin,
    Json(req): Json<NewPlan>,
) -> Result<(StatusCode, Json<Plan>), AppError> {
    admin.may_change()?;
    let plain = |s: &str| {
        (1..=48).contains(&s.len())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
    };
    if req.name.trim().is_empty() || req.name.chars().count() > 100 {
        return Err(bad("name", "1 to 100 characters."));
    }
    if req.features.len() > 64 || !req.features.iter().all(|f| plain(f)) {
        return Err(bad(
            "features",
            "Up to 64 names of letters, digits, _ and -.",
        ));
    }
    if req.limits.len() > 32
        || !req.limits.keys().all(|k| plain(k))
        || req.limits.values().any(|v| *v < -1 || *v > 1_000_000_000)
    {
        return Err(bad(
            "limits",
            "Up to 32 limits, each -1 (none) to 1000000000.",
        ));
    }
    if !(1..=10_000).contains(&req.max_activations) {
        return Err(bad("max_activations", "1 to 10000."));
    }
    if req.valid_days.is_some_and(|d| !(1..=36_500).contains(&d)) {
        return Err(bad("valid_days", "1 to 36500."));
    }
    let limits = serde_json::to_value(&req.limits).map_err(AppError::internal)?;
    let made: Result<Plan, sqlx::Error> = sqlx::query_as(
        "INSERT INTO plans (product_id, name, features, limits, max_activations, valid_days)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, product_id, name, features, limits, max_activations, valid_days",
    )
    .bind(req.product_id)
    .bind(req.name.trim())
    .bind(&req.features)
    .bind(limits)
    .bind(req.max_activations)
    .bind(req.valid_days)
    .fetch_one(&state.db)
    .await;
    let made = made.map_err(|e| {
        match e
            .as_database_error()
            .and_then(|d| d.code().map(|c| c.to_string()))
            .as_deref()
        {
            Some("23505") => {
                AppError::conflict("plan_exists", "This product has a plan of this name.")
            }
            Some("23503") => bad("product_id", "No such product."),
            _ => db(e),
        }
    })?;
    audit(
        &state.db,
        &admin.email,
        "plan.created",
        Some(&made.id.to_string()),
        serde_json::json!({}),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(made)))
}

// ---------------------------------------------------------------- licenses

/// A license.
#[derive(Serialize, FromRow)]
pub struct License {
    /// Id.
    pub id: Uuid,
    /// The customer.
    pub customer_id: Uuid,
    /// The plan.
    pub plan_id: Uuid,
    /// The last part of its key.
    pub key_hint: String,
    /// `active`, `suspended` or `revoked`.
    pub status: String,
    /// When it ends (Unix seconds; none: no end).
    pub expires_at: Option<i64>,
    /// Why it was suspended or revoked.
    pub status_note: Option<String>,
    /// Created (Unix seconds).
    pub created_at: i64,
    /// Installations holding it now.
    pub active_activations: i64,
    /// The domains it is bound to.
    pub bound_domains: Vec<String>,
    /// The host names it is bound to.
    pub bound_hostnames: Vec<String>,
    /// The public addresses it is bound to.
    pub bound_ips: Vec<String>,
}

macro_rules! license {
    ($($tail:literal),*) => {
        concat!("SELECT l.id, l.customer_id, l.plan_id, l.key_hint, l.status,
    extract(epoch FROM l.expires_at)::bigint AS expires_at, l.status_note,
    extract(epoch FROM l.created_at)::bigint AS created_at,
    l.bound_domains, l.bound_hostnames, l.bound_ips,
    (SELECT count(*) FROM activations a WHERE a.license_id = l.id AND a.deactivated_at IS NULL) AS active_activations
    FROM licenses l"$(, $tail)*)
    };
}

/// A license to issue.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewLicense {
    /// The customer.
    pub customer_id: Uuid,
    /// The plan.
    pub plan_id: Uuid,
    /// When it ends (Unix seconds); when absent, the plan's `valid_days` from now, or never.
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// The domains it may be used at (the panel's domain; sub-domains fit). At least one.
    pub domains: Vec<String>,
    /// The host names of the machines it may run on. At least one.
    pub hostnames: Vec<String>,
    /// The public addresses the installation is seen from. At least one.
    pub ips: Vec<String>,
}

/// Where a license may be used, replacing what it was bound to.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewBinding {
    /// The domains (at least one).
    pub domains: Vec<String>,
    /// The host names (at least one).
    pub hostnames: Vec<String>,
    /// The public addresses (at least one).
    pub ips: Vec<String>,
}

/// A license made, with its key (this once).
#[derive(Serialize)]
pub struct IssuedLicense {
    /// The license.
    pub license: License,
    /// The key the customer types in (never shown again).
    pub key: String,
}

/// Which licenses.
#[derive(Deserialize)]
pub struct LicenseQuery {
    /// Only this customer's.
    pub customer_id: Option<Uuid>,
    /// Only this status.
    pub status: Option<String>,
}

const KEY_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// A new license key: `DHP-` and four groups of five, 100 bits.
fn new_key() -> Result<String, AppError> {
    let bytes = random_bytes(20)?;
    let groups: Vec<String> = bytes
        .chunks(5)
        .map(|c| {
            c.iter()
                .map(|b| char::from(KEY_ALPHABET[usize::from(*b) % 32]))
                .collect()
        })
        .collect();
    Ok(format!("DHP-{}", groups.join("-")))
}

async fn list_licenses(
    State(state): State<LaState>,
    _a: Admin,
    Query(q): Query<LicenseQuery>,
) -> Result<Json<Vec<License>>, AppError> {
    sqlx::query_as(license!(
        " WHERE ($1::uuid IS NULL OR l.customer_id = $1) AND ($2::text IS NULL OR l.status = $2)
         ORDER BY l.created_at DESC LIMIT 500"
    ))
    .bind(q.customer_id)
    .bind(q.status)
    .fetch_all(&state.db)
    .await
    .map(Json)
    .map_err(db)
}

async fn fetch_license(state: &LaState, id: Uuid) -> Result<License, AppError> {
    sqlx::query_as(license!(" WHERE l.id = $1"))
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .map_err(db)?
        .ok_or_else(|| AppError::not_found("license"))
}

async fn add_license(
    State(state): State<LaState>,
    admin: Admin,
    Json(req): Json<NewLicense>,
) -> Result<(StatusCode, Json<IssuedLicense>), AppError> {
    admin.may_change()?;
    let binding = crate::binding::validate(&req.domains, &req.hostnames, &req.ips)?;
    let key = new_key()?;
    let hint = key.rsplit('-').next().unwrap_or_default().to_owned();
    let id: Result<Uuid, sqlx::Error> = sqlx::query_scalar(
        "INSERT INTO licenses (customer_id, plan_id, key_hash, key_hint, expires_at,
                               bound_domains, bound_hostnames, bound_ips)
         SELECT $1, p.id, $3, $4,
                COALESCE(to_timestamp($5::bigint), now() + make_interval(days => p.valid_days)),
                $6, $7, $8
         FROM plans p WHERE p.id = $2
         RETURNING id",
    )
    .bind(req.customer_id)
    .bind(req.plan_id)
    .bind(sha256(&key))
    .bind(&hint)
    .bind(req.expires_at)
    .bind(&binding.domains)
    .bind(&binding.hostnames)
    .bind(&binding.ips)
    .fetch_one(&state.db)
    .await;
    let id = id.map_err(|e| match e {
        sqlx::Error::RowNotFound => bad("plan_id", "No such plan."),
        other
            if other
                .as_database_error()
                .and_then(|d| d.code().map(|c| c.to_string()))
                .as_deref()
                == Some("23503") =>
        {
            bad("customer_id", "No such customer.")
        }
        other => db(other),
    })?;
    audit(
        &state.db,
        &admin.email,
        "license.issued",
        Some(&id.to_string()),
        serde_json::json!({
            "domains": binding.domains,
            "hostnames": binding.hostnames,
            "ips": binding.ips,
        }),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(IssuedLicense {
            license: fetch_license(&state, id).await?,
            key,
        }),
    ))
}

async fn set_binding(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<NewBinding>,
) -> Result<Json<License>, AppError> {
    admin.may_change()?;
    let binding = crate::binding::validate(&req.domains, &req.hostnames, &req.ips)?;
    let changed = sqlx::query(
        "UPDATE licenses SET bound_domains = $2, bound_hostnames = $3, bound_ips = $4, updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(&binding.domains)
    .bind(&binding.hostnames)
    .bind(&binding.ips)
    .execute(&state.db)
    .await
    .map_err(db)?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::not_found("license"));
    }
    audit(
        &state.db,
        &admin.email,
        "license.rebound",
        Some(&id.to_string()),
        serde_json::json!({
            "domains": binding.domains,
            "hostnames": binding.hostnames,
            "ips": binding.ips,
        }),
    )
    .await?;
    Ok(Json(fetch_license(&state, id).await?))
}

/// A license with the installations that hold it.
#[derive(Serialize)]
pub struct LicenseDetail {
    /// The license.
    pub license: License,
    /// Who holds it.
    pub activations: Vec<Activation>,
}

async fn get_license(
    State(state): State<LaState>,
    _a: Admin,
    Path(id): Path<Uuid>,
) -> Result<Json<LicenseDetail>, AppError> {
    let license = fetch_license(&state, id).await?;
    let activations = activations_of(&state, Some(id)).await?;
    Ok(Json(LicenseDetail {
        license,
        activations,
    }))
}

/// Why a status changed.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// A line of explanation.
    #[serde(default)]
    pub note: Option<String>,
}

async fn set_status(
    state: &LaState,
    admin: &Admin,
    id: Uuid,
    status: &str,
    note: Option<&str>,
    from: &[&str],
) -> Result<Json<License>, AppError> {
    admin.may_change()?;
    if note.is_some_and(|n| n.chars().count() > 500) {
        return Err(bad("note", "At most 500 characters."));
    }
    let from: Vec<String> = from.iter().map(|s| (*s).to_owned()).collect();
    let mut tx = state.db.begin().await.map_err(db)?;
    let changed = sqlx::query(
        "UPDATE licenses SET status = $2, status_note = $3, updated_at = now() WHERE id = $1 AND status = ANY($4)",
    )
    .bind(id)
    .bind(status)
    .bind(note)
    .bind(&from)
    .execute(&mut *tx)
    .await
    .map_err(db)?
    .rows_affected();
    if changed == 0 {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM licenses WHERE id = $1)")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        return Err(if exists {
            AppError::conflict(
                "wrong_state",
                format!(
                    "A license that is {} cannot be made {status}.",
                    "in this state"
                ),
            )
        } else {
            AppError::not_found("license")
        });
    }
    if status == "revoked" {
        sqlx::query("INSERT INTO revocations (license_id, reason, revoked_by) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(note.unwrap_or(""))
            .bind(admin.id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    audit(
        &mut *tx,
        &admin.email,
        &format!("license.{status}"),
        Some(&id.to_string()),
        serde_json::json!({ "note": note }),
    )
    .await?;
    tx.commit().await.map_err(db)?;
    Ok(Json(fetch_license(state, id).await?))
}

async fn suspend(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    body: Option<Json<Note>>,
) -> Result<Json<License>, AppError> {
    let note = body.and_then(|b| b.0.note);
    set_status(
        &state,
        &admin,
        id,
        "suspended",
        note.as_deref(),
        &["active"],
    )
    .await
}

async fn resume(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
) -> Result<Json<License>, AppError> {
    set_status(&state, &admin, id, "active", None, &["suspended"]).await
}

async fn revoke(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    body: Option<Json<Note>>,
) -> Result<Json<License>, AppError> {
    let note = body.and_then(|b| b.0.note);
    set_status(
        &state,
        &admin,
        id,
        "revoked",
        note.as_deref(),
        &["active", "suspended"],
    )
    .await
}

/// A new end date.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Renew {
    /// The new end (Unix seconds)…
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// …or this many days more than the current end (or than now, if that has passed).
    #[serde(default)]
    pub days: Option<i32>,
}

async fn renew(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Json(req): Json<Renew>,
) -> Result<Json<License>, AppError> {
    admin.may_change()?;
    let changed = match (req.expires_at, req.days) {
        (Some(at), None) if at > 0 => sqlx::query("UPDATE licenses SET expires_at = to_timestamp($2::bigint), updated_at = now() WHERE id = $1 AND status <> 'revoked'")
            .bind(id)
            .bind(at)
            .execute(&state.db)
            .await,
        (None, Some(days)) if (1..=36_500).contains(&days) => sqlx::query(
            "UPDATE licenses SET expires_at = GREATEST(COALESCE(expires_at, now()), now()) + make_interval(days => $2), updated_at = now()
             WHERE id = $1 AND status <> 'revoked'",
        )
        .bind(id)
        .bind(days)
        .execute(&state.db)
        .await,
        _ => return Err(bad("days", "Give expires_at, or days from 1 to 36500 (one of them).")),
    }
    .map_err(db)?
    .rows_affected();
    if changed == 0 {
        // Nothing renewable: not there, or revoked (a revoked license stays revoked).
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM licenses WHERE id = $1")
                .bind(id)
                .fetch_optional(&state.db)
                .await
                .map_err(db)?;
        return Err(match status {
            None => AppError::not_found("license"),
            Some(_) => AppError::conflict("revoked", "A revoked license cannot be renewed."),
        });
    }
    audit(
        &state.db,
        &admin.email,
        "license.renewed",
        Some(&id.to_string()),
        serde_json::json!({ "expires_at": req.expires_at, "days": req.days }),
    )
    .await?;
    Ok(Json(fetch_license(&state, id).await?))
}

// ---------------------------------------------------------------- activations

/// An installation holding a license.
#[derive(Serialize, FromRow)]
pub struct Activation {
    /// Id.
    pub id: Uuid,
    /// The license.
    pub license_id: Uuid,
    /// The installation.
    pub installation_id: Uuid,
    /// Its host name, as it said.
    pub hostname: Option<String>,
    /// Since (Unix seconds).
    pub activated_at: i64,
    /// When it let go (Unix seconds).
    pub deactivated_at: Option<i64>,
    /// The last heartbeat (Unix seconds).
    pub last_heartbeat_at: Option<i64>,
    /// The last refresh (Unix seconds).
    pub last_refresh_at: Option<i64>,
    /// What the last heartbeat said.
    pub heartbeat_info: serde_json::Value,
}

async fn activations_of(
    state: &LaState,
    license: Option<Uuid>,
) -> Result<Vec<Activation>, AppError> {
    sqlx::query_as(
        "SELECT a.id, a.license_id, a.installation_id, i.hostname,
                extract(epoch FROM a.activated_at)::bigint AS activated_at,
                extract(epoch FROM a.deactivated_at)::bigint AS deactivated_at,
                extract(epoch FROM a.last_heartbeat_at)::bigint AS last_heartbeat_at,
                extract(epoch FROM a.last_refresh_at)::bigint AS last_refresh_at,
                a.heartbeat_info
         FROM activations a JOIN installations i ON i.id = a.installation_id
         WHERE ($1::uuid IS NULL OR a.license_id = $1) ORDER BY a.activated_at DESC LIMIT 500",
    )
    .bind(license)
    .fetch_all(&state.db)
    .await
    .map_err(db)
}

/// Which activations.
#[derive(Deserialize)]
pub struct ActivationQuery {
    /// Only this license's.
    pub license_id: Option<Uuid>,
}

async fn list_activations(
    State(state): State<LaState>,
    _a: Admin,
    Query(q): Query<ActivationQuery>,
) -> Result<Json<Vec<Activation>>, AppError> {
    Ok(Json(activations_of(&state, q.license_id).await?))
}

async fn deactivate(
    State(state): State<LaState>,
    admin: Admin,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    admin.may_change()?;
    let changed = sqlx::query(
        "UPDATE activations SET deactivated_at = now() WHERE id = $1 AND deactivated_at IS NULL",
    )
    .bind(id)
    .execute(&state.db)
    .await
    .map_err(db)?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::not_found("activation"));
    }
    audit(
        &state.db,
        &admin.email,
        "activation.deactivated",
        Some(&id.to_string()),
        serde_json::json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------- keys

/// A signing key (its public half).
#[derive(Serialize, FromRow)]
pub struct KeyInfo {
    /// The key id.
    pub kid: String,
    /// `license` or `release`.
    pub purpose: String,
    /// `active` or `retired`.
    pub status: String,
    /// The public key (base64).
    pub public_key: String,
    /// Created (Unix seconds).
    pub created_at: i64,
}

async fn list_keys(
    State(state): State<LaState>,
    _a: Admin,
) -> Result<Json<Vec<KeyInfo>>, AppError> {
    sqlx::query_as(
        "SELECT kid, purpose, status, encode(public_key, 'base64') AS public_key,
                extract(epoch FROM created_at)::bigint AS created_at
         FROM signing_keys ORDER BY created_at DESC",
    )
    .fetch_all(&state.db)
    .await
    .map(Json)
    .map_err(db)
}

/// A new key id.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rotate {
    /// The id of the new key (`license-2`).
    pub kid: String,
}

async fn rotate_keys(
    State(state): State<LaState>,
    admin: Admin,
    Json(req): Json<Rotate>,
) -> Result<(StatusCode, Json<KeyInfo>), AppError> {
    admin.may_change()?;
    if !(1..=64).contains(&req.kid.len())
        || !req
            .kid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
    {
        return Err(bad("kid", "1 to 64 letters, digits, - and _."));
    }
    let path = state.settings.key_dir.join(format!("{}.key", req.kid));
    let key = FileSigningKey::generate_to_file(&path, &req.kid).map_err(|e| match e {
        builtpanel_license::KeyError::File(_) => {
            AppError::conflict("key_exists", "A key with this id is there already.")
        }
        other => AppError::internal(other),
    })?;
    crate::register_key(&state.db, &key).await?;
    *state
        .signer
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = std::sync::Arc::new(key);
    audit(
        &state.db,
        &admin.email,
        "key.rotated",
        Some(&req.kid),
        serde_json::json!({}),
    )
    .await?;
    let info: KeyInfo = sqlx::query_as(
        "SELECT kid, purpose, status, encode(public_key, 'base64') AS public_key,
                extract(epoch FROM created_at)::bigint AS created_at FROM signing_keys WHERE kid = $1",
    )
    .bind(&req.kid)
    .fetch_one(&state.db)
    .await
    .map_err(db)?;
    Ok((StatusCode::CREATED, Json(info)))
}

// ---------------------------------------------------------------- audit

/// An audit entry.
#[derive(Serialize, FromRow)]
pub struct AuditEntry {
    /// Id.
    pub id: i64,
    /// When (Unix seconds).
    pub at: i64,
    /// Who.
    pub actor: String,
    /// What.
    pub action: String,
    /// On what.
    pub target: Option<String>,
    /// More.
    pub detail: serde_json::Value,
}

async fn list_audit(
    State(state): State<LaState>,
    _a: Admin,
) -> Result<Json<Vec<AuditEntry>>, AppError> {
    sqlx::query_as(
        "SELECT id, extract(epoch FROM at)::bigint AS at, actor, action, target, detail
         FROM audit_logs ORDER BY id DESC LIMIT 200",
    )
    .fetch_all(&state.db)
    .await
    .map(Json)
    .map_err(db)
}
