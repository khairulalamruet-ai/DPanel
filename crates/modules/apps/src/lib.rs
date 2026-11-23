//! Which application runs where (T12.01).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/client/apps[?domain_id=]` | the owner |
//! | GET | `/api/v1/client/apps/{id}` | the owner |
//! | POST | `/api/v1/client/apps/wordpress` | the owner; the `databases` feature |
//! | GET | `/api/v1/client/apps/catalog` | the owner |
//! | POST | `/api/v1/client/apps/install` | the owner (any application of the catalog) |
//! | GET, POST | `/api/v1/client/cache` | the owner; the `cache` feature |
//! | DELETE | `/api/v1/client/cache/{kind}` | the owner |
//! | GET | `/api/v1/client/apps/{id}/wordpress` | the owner (versions, updates) |
//! | POST | `/api/v1/client/apps/{id}/wordpress/{plugins,themes,core}/update` | the owner |
//! | POST | `/api/v1/client/apps/{id}/wordpress/{maintenance,admin-password}` | the owner |
//! | GET | `/api/v1/client/apps/{id}/wordpress/scan` | the owner (core checksums, safety settings) |
//! | POST | `/api/v1/client/apps/{id}/wordpress/harden` | the owner |
//! | GET | `/api/v1/client/apps/{id}/operations/{op}` | the owner (the answer of a slow action) |
//! | GET | `/api/v1/admin/tenants/{id}/apps` | `admin.apps.read` |
//!
//! An application is a catalog kind (WordPress, Joomla, a Node.js app, ...) at a place: a
//! domain and a path under it. There is one application per place, so the panel always knows
//! what answers on `example.com/blog`. The installers (T12.02, T12.05) create the rows and
//! move them from `installing` to `active` or `failed`; this module shows them.

pub mod containers;
pub mod gitdeploy;
pub mod marketplace;
pub mod runtime;

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    routing::get,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_database::{
    accounts::DbAccountKind,
    apps as store, databases as dbstore,
    operations::{self, EnqueueError, NewOperation},
    plans,
    sqlx::PgPool,
    tenants, users,
};
use builtpanel_domain::TenantUsername;
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::{DbEngine, DbPassword, Operation};
use builtpanel_rbac::{Principal, Require, permissions::AppsRead};
use builtpanel_session::SessionStore;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/apps/module.toml");

/// What the module's handlers share.
#[derive(Clone)]
pub struct AppsState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// Seals the passwords an install needs.
    pub master_key: MasterKey,
    /// How long a request waits for the node before answering `202`.
    pub wait: std::time::Duration,
    /// The applications that can be installed.
    pub catalog: Arc<builtpanel_catalog::Catalog>,
}

/// How long a request waits for the node by default.
pub const DEFAULT_WAIT: std::time::Duration = std::time::Duration::from_secs(20);

impl FromRef<AppsState> for PgPool {
    fn from_ref(s: &AppsState) -> Self {
        s.db.clone()
    }
}

impl FromRef<AppsState> for SessionStore {
    fn from_ref(s: &AppsState) -> Self {
        s.sessions.clone()
    }
}

/// The routes of the apps module that need no session (the webhook of a git deploy).
pub fn public_router() -> Router<AppsState> {
    gitdeploy::public_router()
}

/// Routes for hosting accounts.
pub fn client_router() -> Router<AppsState> {
    Router::new()
        .merge(runtime::router())
        .merge(containers::client_router())
        .merge(marketplace::client_router())
        .merge(gitdeploy::router())
        .route("/api/v1/client/apps", get(client_list))
        .route(
            "/api/v1/client/apps/{id}",
            get(client_get).delete(client_forget),
        )
        .route(
            "/api/v1/client/apps/wordpress",
            axum::routing::post(install_wordpress),
        )
        .route("/api/v1/client/apps/catalog", get(catalog_list))
        .route("/api/v1/client/cache", get(cache_list).post(cache_set))
        .route(
            "/api/v1/client/cache/{kind}",
            axum::routing::delete(cache_remove),
        )
        .route(
            "/api/v1/client/apps/install",
            axum::routing::post(install_app),
        )
        .route("/api/v1/client/apps/{id}/wordpress", get(wp_overview))
        .route(
            "/api/v1/client/apps/{id}/wordpress/plugins/update",
            axum::routing::post(wp_update_plugins),
        )
        .route(
            "/api/v1/client/apps/{id}/wordpress/themes/update",
            axum::routing::post(wp_update_themes),
        )
        .route(
            "/api/v1/client/apps/{id}/wordpress/core/update",
            axum::routing::post(wp_update_core),
        )
        .route(
            "/api/v1/client/apps/{id}/wordpress/maintenance",
            axum::routing::post(wp_maintenance),
        )
        .route(
            "/api/v1/client/apps/{id}/wordpress/admin-password",
            axum::routing::post(wp_admin_password),
        )
        .route("/api/v1/client/apps/{id}/wordpress/scan", get(wp_scan))
        .route(
            "/api/v1/client/apps/{id}/wordpress/harden",
            axum::routing::post(wp_harden),
        )
        .route(
            "/api/v1/client/apps/{id}/operations/{op}",
            get(wp_operation),
        )
}

/// Routes for administrators.
pub fn admin_router() -> Router<AppsState> {
    Router::new()
        .route("/api/v1/admin/tenants/{id}/apps", get(admin_list))
        .merge(containers::admin_router())
        .merge(marketplace::admin_router())
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

/// One application.
#[derive(Debug, Serialize, ToSchema)]
pub struct AppView {
    /// Id.
    pub id: Uuid,
    /// The domain it is served on (`null`: the domain was removed).
    pub domain_id: Option<Uuid>,
    /// The domain's name.
    pub fqdn: Option<String>,
    /// The path under the domain (`/`, `/blog`).
    pub path: String,
    /// The catalog slug (`wordpress`).
    pub kind: String,
    /// What the owner calls it.
    pub name: String,
    /// The version installed.
    pub version: Option<String>,
    /// `installing`, `active`, `failed` or `removing`.
    pub status: String,
    /// The database it uses.
    pub database_id: Option<Uuid>,
    /// Why an install failed.
    pub error: Option<String>,
    /// Created (Unix seconds).
    pub created_at: i64,
}

/// The applications.
#[derive(Debug, Serialize, ToSchema)]
pub struct AppList {
    /// Oldest first.
    pub items: Vec<AppView>,
}

fn view(a: store::App) -> AppView {
    AppView {
        id: a.id,
        domain_id: a.domain_id,
        fqdn: a.fqdn,
        path: a.path,
        kind: a.kind,
        name: a.name,
        version: a.version,
        status: a.status,
        database_id: a.database_id,
        error: a.error,
        created_at: a.created_at,
    }
}

/// Which applications to list.
#[derive(Debug, Deserialize, IntoParams)]
pub struct ListQuery {
    /// Only those served on this domain.
    pub domain_id: Option<Uuid>,
}

async fn client_tenant(state: &AppsState, who: &Principal) -> Result<tenants::Tenant, AppError> {
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
    tenants::get(&state.db, owned.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("user"))
}

/// The account's applications.
#[utoipa::path(get, path = "/api/v1/client/apps", tag = "apps", operation_id = "clientListApps",
    security(("session" = [])), params(ListQuery),
    responses((status = 200, description = "The applications", body = AppList),
              (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_list(
    State(state): State<AppsState>,
    who: Principal,
    Query(q): Query<ListQuery>,
) -> Result<Json<AppList>, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    let items = store::list(&state.db, tenant.id, q.domain_id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(view)
        .collect();
    Ok(Json(AppList { items }))
}

/// One of the account's applications.
#[utoipa::path(get, path = "/api/v1/client/apps/{id}", tag = "apps", operation_id = "clientGetApp",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id")),
    responses((status = 200, description = "The application", body = AppView),
              (status = 404, description = "Not the account's", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_get(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<AppView>, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    store::get(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .map(|a| Json(view(a)))
        .ok_or_else(|| AppError::not_found("app"))
}

/// Stop tracking an application. Its files and its database stay in the account (they are
/// removed in Files and Databases); the place becomes free for another installation.
#[utoipa::path(delete, path = "/api/v1/client/apps/{id}", tag = "apps", operation_id = "clientForgetApp",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id")),
    responses((status = 204, description = "No longer tracked"),
              (status = 404, description = "Not the account's", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`app_installing`: wait until the installation ends", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn client_forget(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    let app = store::get(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("app"))?;
    if app.status == "installing" || app.status == "removing" {
        return Err(AppError::conflict(
            "app_installing",
            "The application is being installed; try again when that is done.",
        ));
    }
    store::remove(&state.db, tenant.id, id)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("app.removed", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("app", id)
            .metadata(serde_json::json!({ "kind": app.kind, "name": app.name })),
    )
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A hosting account's applications.
#[utoipa::path(get, path = "/api/v1/admin/tenants/{id}/apps", tag = "apps", operation_id = "adminListApps",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Hosting account id")),
    responses((status = 200, description = "The applications", body = AppList),
              (status = 404, description = "No such hosting account", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn admin_list(
    State(state): State<AppsState>,
    _who: Require<AppsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<AppList>, AppError> {
    tenants::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("tenant"))?;
    let items = store::list(&state.db, id, None)
        .await
        .map_err(internal)?
        .into_iter()
        .map(view)
        .collect();
    Ok(Json(AppList { items }))
}

/// A WordPress site to install.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallWordPress {
    /// The domain it is served on (one of the account's).
    pub domain_id: Uuid,
    /// The path under the domain (`/` when absent, `/blog`).
    #[serde(default)]
    pub path: Option<String>,
    /// The site's title.
    pub title: String,
    /// The first administrator's login (3 to 60: letters, digits, `_`, `.`, `-`).
    pub admin_user: String,
    /// Their email address.
    pub admin_email: String,
    /// Their password (12 to 128 characters).
    pub admin_password: String,
    /// A locale such as `bn_BD` (`en_US` when absent).
    #[serde(default)]
    pub locale: Option<String>,
    /// Plugins of wordpress.org to install and switch on (at most 10).
    #[serde(default)]
    pub plugins: Vec<String>,
}

impl std::fmt::Debug for InstallWordPress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallWordPress")
            .field("domain_id", &self.domain_id)
            .field("path", &self.path)
            .field("admin_password", &"..")
            .finish_non_exhaustive()
    }
}

const OPERATION_TTL_SECS: i32 = 6 * 3600;
const OPERATION_ATTEMPTS: i32 = 5;

fn plain(text: &str, min: usize, max: usize, extra: &str) -> bool {
    (min..=max).contains(&text.len())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

/// A strong password made here (about 100 bits).
fn generate_password() -> String {
    loop {
        let a = Uuid::new_v4().simple().to_string();
        let b = Uuid::new_v4().simple().to_string();
        let candidate = format!("{}-{}", a[..10].to_uppercase(), &b[..14]);
        if builtpanel_domain::databases::check_password(&candidate).is_ok() {
            return candidate;
        }
    }
}

fn bad(field: &'static str, message: &str) -> AppError {
    AppError::validation(vec![FieldError::new(field, "invalid", message)])
}

/// Install WordPress on a domain (and path) of the account: a database and a user of its own
/// are made, then WordPress is downloaded, checked, configured and installed on the node. The
/// answer is the application, `installing`; it turns `active` (or `failed`, with the reason)
/// within a minute or two.
#[utoipa::path(post, path = "/api/v1/client/apps/wordpress", tag = "apps", operation_id = "clientInstallWordPress",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = InstallWordPress,
    responses((status = 202, description = "Queued", body = AppView),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`place_taken`, `limit_reached` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A value that is not valid, or a domain that is not the account's", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn install_wordpress(
    State(state): State<AppsState>,
    who: Principal,
    Json(req): Json<InstallWordPress>,
) -> Result<(StatusCode, Json<AppView>), AppError> {
    let tenant = client_tenant(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can install applications.",
        ));
    }
    let plan = plans::get(&state.db, tenant.plan_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("the plan of a tenant vanished")))?;
    if !plans::includes_feature(&state.db, plan.id, "databases")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Databases are not included in this plan.",
        ));
    }
    // What was asked for.
    let path = req
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or("/");
    let folder = path.trim_start_matches('/');
    if !path.starts_with('/')
        || path.len() > 200
        || (!folder.is_empty()
            && !folder
                .split('/')
                .all(|part| plain(part, 1, 100, "._-") && part != "." && part != ".."))
    {
        return Err(bad("path", "A path such as / or /blog."));
    }
    let title = req.title.trim();
    if title.is_empty() || title.chars().count() > 100 || title.chars().any(char::is_control) {
        return Err(bad("title", "1 to 100 characters."));
    }
    if !plain(&req.admin_user, 3, 60, "_.-") {
        return Err(bad("admin_user", "3 to 60 letters, digits, _ . -"));
    }
    let email = req.admin_email.trim();
    if !email.contains('@') || !plain(email, 3, 200, "@._+-") {
        return Err(bad("admin_email", "An email address."));
    }
    if req.admin_password.chars().count() < 12
        || req.admin_password.len() > 128
        || req.admin_password.chars().any(char::is_control)
    {
        return Err(bad("admin_password", "12 to 128 characters."));
    }
    if req.locale.as_deref().is_some_and(|l| !plain(l, 2, 10, "_"))
        || req.plugins.len() > 10
        || req.plugins.iter().any(|p| !plain(p, 1, 100, "-"))
    {
        return Err(bad(
            "plugins",
            "A locale such as bn_BD; up to 10 plugin slugs.",
        ));
    }
    let fqdn = builtpanel_database::sqlx::query_scalar::<_, String>(
        "SELECT fqdn FROM domains WHERE id = $1 AND tenant_id = $2",
    )
    .bind(req.domain_id)
    .bind(tenant.id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| bad("domain_id", "Not one of your domains."))?;
    let url = if folder.is_empty() {
        format!("https://{fqdn}")
    } else {
        format!("https://{fqdn}/{folder}")
    };
    let tenant_name = TenantUsername::parse(&tenant.username).map_err(internal)?;
    let stem = format!("{}_wp", tenant_name.as_str());
    let number = store::next_name_number(&state.db, tenant.id, &stem)
        .await
        .map_err(internal)?;
    let name = format!("{stem}{number}");
    if name.len() > builtpanel_domain::databases::MAX_NAME_LEN {
        return Err(bad(
            "domain_id",
            "The account's name is too long for a database name.",
        ));
    }
    let settings = serde_json::json!({
        "stage": "setup",
        "folder": folder,
        "url": url,
        "title": title,
        "admin_user": req.admin_user,
        "admin_email": email,
        "locale": req.locale,
        "plugins": req.plugins,
        "database": name,
    });
    let app_id = store::create(
        &state.db,
        &store::NewApp {
            tenant_id: tenant.id,
            domain_id: Some(req.domain_id),
            path: if folder.is_empty() { "/" } else { path },
            kind: "wordpress",
            name: title,
            version: None,
            database_id: None,
            settings: &settings,
        },
    )
    .await
    .map_err(|e| match e {
        store::CreateError::Taken => {
            AppError::conflict("place_taken", "Something is installed there already.")
        }
        store::CreateError::NotOwned => bad("domain_id", "Not one of your domains."),
        store::CreateError::Database(db) => internal(db),
    })?;
    let result = queue_setup(
        &state,
        &who,
        &tenant,
        &plan,
        app_id,
        Some(&name),
        Some(&req.admin_password),
        "wordpress",
    )
    .await;
    if let Err(err) = result {
        let _ = store::set_status(
            &state.db,
            app_id,
            "failed",
            None,
            Some("could not be set up"),
        )
        .await;
        return Err(err);
    }
    let app = store::get(&state.db, tenant.id, app_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("app"))?;
    Ok((StatusCode::ACCEPTED, Json(view(app))))
}

/// The database and its user (when the application needs one), the sealed passwords and the
/// first operations, in one transaction.
#[allow(clippy::too_many_arguments)]
async fn queue_setup(
    state: &AppsState,
    who: &Principal,
    tenant: &tenants::Tenant,
    plan: &plans::Plan,
    app_id: Uuid,
    name: Option<&str>,
    admin_password: Option<&str>,
    kind: &str,
) -> Result<(), AppError> {
    let limit = |key: &str| {
        let value = plan
            .limits
            .get(key)
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(10);
        (value >= 0).then_some(value)
    };
    let seal = |context: Vec<u8>, text: &str| {
        state
            .master_key
            .seal(&context, text.as_bytes())
            .map_err(internal)
    };
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, tenant.id)
        .await
        .map_err(internal)?;
    if let Some(name) = name {
        if let Some(max) = limit("max_databases")
            && dbstore::count(&mut tx, tenant.id).await.map_err(internal)? >= max
        {
            return Err(AppError::conflict(
                "limit_reached",
                format!("This plan allows {max} databases."),
            ));
        }
        if let Some(max) = limit("max_db_users")
            && dbstore::count_users(&mut tx, tenant.id)
                .await
                .map_err(internal)?
                >= max
        {
            return Err(AppError::conflict(
                "limit_reached",
                format!("This plan allows {max} database users."),
            ));
        }
        let taken = |e: dbstore::AddError| match e {
            dbstore::AddError::Taken => {
                AppError::conflict("name_taken", "The database name is taken.")
            }
            dbstore::AddError::Database(db) => internal(db),
        };
        let dedicated = match builtpanel_database::tenant_db::dedicated_for(
            &state.db,
            tenant.id,
            &tenant.username,
            "mysql",
        )
        .await
        .map_err(internal)?
        {
            Ok(d) => d,
            Err(_) => {
                return Err(AppError::conflict(
                    "database_server_moving",
                    "Your databases are being moved to another server; try again in a few minutes.",
                ));
            }
        };
        let db = dbstore::create(&mut tx, tenant.id, name, "mysql")
            .await
            .map_err(taken)?;
        let user = dbstore::create_user(&mut tx, tenant.id, name, "mysql")
            .await
            .map_err(taken)?;
        let password = generate_password();
        dbstore::keep_password(
            &mut tx,
            user.id,
            &seal(dbstore::password_context(user.id), &password)?,
        )
        .await
        .map_err(internal)?;
        store::keep_secret(
            &mut tx,
            app_id,
            "db_password",
            &seal(store::secret_context(app_id, "db_password"), &password)?,
        )
        .await
        .map_err(internal)?;
        for (op, key) in [
            (
                Operation::CreateDatabase {
                    name: name.to_owned(),
                    engine: DbEngine::Mysql,
                    dedicated: dedicated.clone(),
                },
                format!("dbcreate:{}", db.id.simple()),
            ),
            (
                Operation::UpsertDbUser {
                    username: tenant.username.clone(),
                    name: name.to_owned(),
                    password: None::<DbPassword>,
                    engine: DbEngine::Mysql,
                    dedicated: dedicated.clone(),
                },
                format!("dbuser:{}:{}", user.id.simple(), Uuid::now_v7().simple()),
            ),
        ] {
            let payload = serde_json::to_value(&op).map_err(internal)?;
            operations::enqueue_in(
                &mut tx,
                &NewOperation {
                    node_id: tenant.node_id,
                    kind: op.type_name(),
                    tenant_id: Some(tenant.id),
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
                    "The node of this account no longer exists.",
                ),
                other => internal(other),
            })?;
        }
        builtpanel_database::sqlx::query(
            "UPDATE apps SET database_id = $2, settings = settings || jsonb_build_object('db_id', $2::text, 'user_id', $3::text) WHERE id = $1",
        )
        .bind(app_id)
        .bind(db.id)
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    }
    if let Some(password) = admin_password {
        store::keep_secret(
            &mut tx,
            app_id,
            "admin_password",
            &seal(store::secret_context(app_id, "admin_password"), password)?,
        )
        .await
        .map_err(internal)?;
    }
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("app.install_requested", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("app", app_id)
            .metadata(serde_json::json!({ "kind": kind, "database": name })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(())
}

/// An application of the catalog, as people choose it.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogEntry {
    /// The catalog name, `joomla`.
    pub slug: String,
    /// What people see.
    pub name: String,
    /// The version it installs.
    pub version: String,
    /// A line about it.
    pub description: String,
    /// A group, `cms`.
    pub category: String,
    /// It gets a database of its own.
    pub needs_database: bool,
    /// The installer asks for an administrator's login, address and password.
    pub needs_admin: bool,
    /// The folder of the application the domain should be served from (empty: its own).
    pub document_root: String,
}

/// The catalog.
#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogList {
    /// By slug.
    pub items: Vec<CatalogEntry>,
}

/// The applications that can be installed.
#[utoipa::path(get, path = "/api/v1/client/apps/catalog", tag = "apps", operation_id = "clientAppCatalog",
    security(("session" = [])),
    responses((status = 200, description = "The catalog", body = CatalogList),
              (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn catalog_list(
    State(state): State<AppsState>,
    who: Principal,
) -> Result<Json<CatalogList>, AppError> {
    client_tenant(&state, &who).await?;
    let items = state
        .catalog
        .all()
        .map(|d| CatalogEntry {
            slug: d.slug.clone(),
            name: d.name.clone(),
            version: d.version.clone(),
            description: d.description.clone(),
            category: d.category.clone(),
            needs_database: d.needs_database,
            needs_admin: d.needs_admin(),
            document_root: d.document_root.clone(),
        })
        .collect();
    Ok(Json(CatalogList { items }))
}

/// An application of the catalog to install.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallApp {
    /// The catalog name, `joomla`.
    pub slug: String,
    /// The domain it is served on (one of the account's).
    pub domain_id: Uuid,
    /// The path under the domain (`/` when absent).
    #[serde(default)]
    pub path: Option<String>,
    /// The site's title.
    pub title: String,
    /// The administrator's login (when the application has one).
    #[serde(default)]
    pub admin_user: Option<String>,
    /// Their email address.
    #[serde(default)]
    pub admin_email: Option<String>,
    /// Their password (12 to 128 characters).
    #[serde(default)]
    pub admin_password: Option<String>,
}

impl std::fmt::Debug for InstallApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallApp")
            .field("slug", &self.slug)
            .field("domain_id", &self.domain_id)
            .field("admin_password", &"..")
            .finish_non_exhaustive()
    }
}

/// Install an application of the catalog. WordPress goes the way of its own installer; the
/// others run the steps of their definition on the node. The answer is the application,
/// `installing`.
#[utoipa::path(post, path = "/api/v1/client/apps/install", tag = "apps", operation_id = "clientInstallApp",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = InstallApp,
    responses((status = 202, description = "Queued", body = AppView),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 404, description = "Not in the catalog", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`place_taken`, `limit_reached` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A value that is not valid", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn install_app(
    State(state): State<AppsState>,
    who: Principal,
    Json(req): Json<InstallApp>,
) -> Result<(StatusCode, Json<AppView>), AppError> {
    let definition = state
        .catalog
        .get(&req.slug)
        .cloned()
        .ok_or_else(|| AppError::not_found("catalog_app"))?;
    if definition.engine == "wordpress" {
        let (Some(admin_user), Some(admin_email), Some(admin_password)) =
            (req.admin_user, req.admin_email, req.admin_password)
        else {
            return Err(bad("admin_user", "WordPress needs an administrator."));
        };
        return install_wordpress(
            State(state),
            who,
            Json(InstallWordPress {
                domain_id: req.domain_id,
                path: req.path,
                title: req.title,
                admin_user,
                admin_email,
                admin_password,
                locale: None,
                plugins: Vec::new(),
            }),
        )
        .await;
    }
    let tenant = client_tenant(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can install applications.",
        ));
    }
    let plan = plans::get(&state.db, tenant.plan_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("the plan of a tenant vanished")))?;
    if definition.needs_database
        && !plans::includes_feature(&state.db, plan.id, "databases")
            .await
            .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Databases are not included in this plan.",
        ));
    }
    let path = req
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or("/");
    let folder = path.trim_start_matches('/');
    if !path.starts_with('/')
        || path.len() > 200
        || (!folder.is_empty()
            && !folder
                .split('/')
                .all(|part| plain(part, 1, 100, "._-") && part != "." && part != ".."))
    {
        return Err(bad("path", "A path such as / or /blog."));
    }
    let title = req.title.trim();
    if title.is_empty()
        || title.chars().count() > 100
        || !title
            .chars()
            .all(|c| c.is_alphanumeric() || " .,_:!&()?-".contains(c))
    {
        return Err(bad(
            "title",
            "1 to 100 letters, digits, spaces and . , _ : ! & ( ) ? -",
        ));
    }
    let (mut admin_user, mut admin_email, mut admin_password) = (None, None, None);
    if definition.needs_admin() {
        let user = req.admin_user.as_deref().unwrap_or_default().to_owned();
        let email = req
            .admin_email
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let password = req.admin_password.clone().unwrap_or_default();
        if !plain(&user, 3, 60, "_.-") {
            return Err(bad("admin_user", "3 to 60 letters, digits, _ . -"));
        }
        if !email.contains('@') || !plain(&email, 3, 200, "@._+-") {
            return Err(bad("admin_email", "An email address."));
        }
        if password.chars().count() < 12
            || password.len() > 128
            || password.chars().any(char::is_control)
        {
            return Err(bad("admin_password", "12 to 128 characters."));
        }
        admin_user = Some(user);
        admin_email = Some(email);
        admin_password = Some(password);
    }
    let fqdn = builtpanel_database::sqlx::query_scalar::<_, String>(
        "SELECT fqdn FROM domains WHERE id = $1 AND tenant_id = $2",
    )
    .bind(req.domain_id)
    .bind(tenant.id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?
    .ok_or_else(|| bad("domain_id", "Not one of your domains."))?;
    let url = if folder.is_empty() {
        format!("https://{fqdn}")
    } else {
        format!("https://{fqdn}/{folder}")
    };
    let db_name = if definition.needs_database {
        let tenant_name = TenantUsername::parse(&tenant.username).map_err(internal)?;
        let stem = format!(
            "{}_{}",
            tenant_name.as_str(),
            definition.slug.replace('-', "")
        );
        let number = store::next_name_number(&state.db, tenant.id, &stem)
            .await
            .map_err(internal)?;
        let name = format!("{stem}{number}");
        if name.len() > builtpanel_domain::databases::MAX_NAME_LEN {
            return Err(bad(
                "domain_id",
                "The account's name is too long for a database name.",
            ));
        }
        Some(name)
    } else {
        None
    };
    let settings = serde_json::json!({
        // A database to wait for, or straight to the installer.
        "stage": if db_name.is_some() { "setup" } else { "grant" },
        "folder": folder,
        "url": url,
        "title": title,
        "admin_user": admin_user,
        "admin_email": admin_email,
        "database": db_name,
        "definition": definition,
    });
    let app_id = store::create(
        &state.db,
        &store::NewApp {
            tenant_id: tenant.id,
            domain_id: Some(req.domain_id),
            path: if folder.is_empty() { "/" } else { path },
            kind: &definition.slug,
            name: title,
            version: None,
            database_id: None,
            settings: &settings,
        },
    )
    .await
    .map_err(|e| match e {
        store::CreateError::Taken => {
            AppError::conflict("place_taken", "Something is installed there already.")
        }
        store::CreateError::NotOwned => bad("domain_id", "Not one of your domains."),
        store::CreateError::Database(db) => internal(db),
    })?;
    let result = queue_setup(
        &state,
        &who,
        &tenant,
        &plan,
        app_id,
        db_name.as_deref(),
        admin_password.as_deref(),
        &definition.slug,
    )
    .await;
    if let Err(err) = result {
        let _ = store::set_status(
            &state.db,
            app_id,
            "failed",
            None,
            Some("could not be set up"),
        )
        .await;
        return Err(err);
    }
    let app = store::get(&state.db, tenant.id, app_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("app"))?;
    Ok((StatusCode::ACCEPTED, Json(view(app))))
}

// ------------------------------------------------------------------ WordPress manager

/// The answer of a WordPress action.
#[derive(Debug, Serialize, ToSchema)]
pub struct WpAnswer {
    /// `running`, `done` or `failed`.
    pub state: String,
    /// The versions and updates (for the overview).
    pub overview: Option<builtpanel_node_protocol::WpOverview>,
    /// What the scan found.
    pub scan: Option<builtpanel_node_protocol::WpScan>,
    /// What WP-CLI said (for the other actions).
    pub output: Option<String>,
    /// Why it failed.
    pub error: Option<String>,
}

/// An action that is still running: ask again with the operation's id.
#[derive(Debug, Serialize, ToSchema)]
pub struct Pending {
    /// The operation to ask about.
    pub operation_id: Uuid,
}

/// A plugin or theme to update (all when absent).
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WpSlug {
    /// Its folder name.
    #[serde(default)]
    pub slug: Option<String>,
}

/// A WordPress version to update to (the latest when absent).
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WpVersion {
    /// Such as `6.8.2`.
    #[serde(default)]
    pub version: Option<String>,
}

/// Maintenance mode on or off.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WpMaintenance {
    /// On or off.
    pub on: bool,
}

/// A new password for a WordPress user.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WpNewPassword {
    /// The user's login.
    pub user: String,
    /// The new password (12 to 128 characters).
    pub password: String,
}

impl std::fmt::Debug for WpNewPassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WpNewPassword")
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

fn answer_of(row: &builtpanel_database::operations::Operation) -> Option<WpAnswer> {
    use builtpanel_database::operations::Status;
    match row.status {
        Status::Succeeded => {
            let output: builtpanel_node_protocol::Output = row
                .output
                .clone()
                .and_then(|o| serde_json::from_value(o).ok())?;
            Some(match output {
                builtpanel_node_protocol::Output::WordPressScan(scan) => WpAnswer {
                    state: "done".into(),
                    overview: None,
                    scan: Some(scan),
                    output: None,
                    error: None,
                },
                builtpanel_node_protocol::Output::WordPressOverview(o) => WpAnswer {
                    state: "done".into(),
                    overview: Some(o),
                    scan: None,
                    output: None,
                    error: None,
                },
                builtpanel_node_protocol::Output::WpCliDone { output } => WpAnswer {
                    state: "done".into(),
                    overview: None,
                    scan: None,
                    output: Some(output),
                    error: None,
                },
                _ => return None,
            })
        }
        Status::Failed => Some(WpAnswer {
            state: "failed".into(),
            overview: None,
            scan: None,
            output: None,
            error: Some(
                row.error
                    .as_ref()
                    .and_then(|e| e.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("the action failed")
                    .to_owned(),
            ),
        }),
        Status::Pending | Status::Dispatched | Status::Retryable => None,
    }
}

/// Runs one WP-CLI action on an active WordPress application of the account and waits a
/// little for the answer.
async fn run_wp(
    state: &AppsState,
    who: &Principal,
    app_id: Uuid,
    command: builtpanel_node_protocol::WpCommand,
    password: Option<&str>,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse as _;
    let tenant = client_tenant(state, who).await?;
    let app = store::get(&state.db, tenant.id, app_id)
        .await
        .map_err(internal)?
        .filter(|a| a.kind == "wordpress")
        .ok_or_else(|| AppError::not_found("app"))?;
    if app.status != "active" {
        return Err(AppError::conflict(
            "app_not_ready",
            "This WordPress is not installed (yet).",
        ));
    }
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can change its applications.",
        ));
    }
    let folder = app.path.trim_start_matches('/').to_owned();
    if let Some(p) = password {
        let sealed = state
            .master_key
            .seal(&store::secret_context(app.id, "admin_reset"), p.as_bytes())
            .map_err(internal)?;
        let mut conn = state.db.acquire().await.map_err(internal)?;
        store::keep_secret(&mut conn, app.id, "admin_reset", &sealed)
            .await
            .map_err(internal)?;
    }
    let op = Operation::RunWpCli {
        app_id: app.id,
        tenant_id: tenant.id,
        username: tenant.username.clone(),
        uid: u32::try_from(tenant.uid).unwrap_or_default(),
        path: folder,
        command: command.clone(),
        password: None::<DbPassword>,
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let queued = operations::enqueue(
        &state.db,
        &NewOperation {
            node_id: tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant.id),
            payload: &payload,
            idempotency_key: &format!("wp:{}", Uuid::new_v4().simple()),
            created_by: Some(who.account_id()),
            ttl_secs: 900,
            max_attempts: 1,
        },
    )
    .await
    .map_err(|e| match e {
        EnqueueError::UnknownNode => AppError::conflict(
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
        if let Some(answer) = answer_of(&row) {
            if password.is_some() {
                let _ = store::forget_secret(&state.db, app.id, "admin_reset").await;
            }
            return Ok(Json(answer).into_response());
        }
        if started.elapsed() >= state.wait {
            return Ok((StatusCode::ACCEPTED, Json(Pending { operation_id: id })).into_response());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// The versions of a WordPress site's core, plugins and themes, what has an update, and
/// whether maintenance mode is on.
#[utoipa::path(get, path = "/api/v1/client/apps/{id}/wordpress", tag = "apps", operation_id = "clientWordPressOverview",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id")),
    responses((status = 200, description = "The overview", body = WpAnswer),
              (status = 202, description = "The node is slow: ask `operations/{operation_id}`", body = Pending),
              (status = 404, description = "Not the account's WordPress", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`app_not_ready`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn wp_overview(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<axum::response::Response, AppError> {
    run_wp(
        &state,
        &who,
        id,
        builtpanel_node_protocol::WpCommand::Overview,
        None,
    )
    .await
}

/// Update one plugin, or all of them.
#[utoipa::path(post, path = "/api/v1/client/apps/{id}/wordpress/plugins/update", tag = "apps", operation_id = "clientWordPressUpdatePlugins",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = WpSlug,
    responses((status = 200, description = "Done", body = WpAnswer), (status = 202, description = "Still running", body = Pending),
              (status = 404, description = "Not the account's WordPress", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn wp_update_plugins(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<WpSlug>,
) -> Result<axum::response::Response, AppError> {
    if req
        .slug
        .as_deref()
        .is_some_and(|s| !plain(s, 1, 100, "-_."))
    {
        return Err(bad("slug", "A plugin's folder name."));
    }
    run_wp(
        &state,
        &who,
        id,
        builtpanel_node_protocol::WpCommand::UpdatePlugins { slug: req.slug },
        None,
    )
    .await
}

/// Update one theme, or all of them.
#[utoipa::path(post, path = "/api/v1/client/apps/{id}/wordpress/themes/update", tag = "apps", operation_id = "clientWordPressUpdateThemes",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = WpSlug,
    responses((status = 200, description = "Done", body = WpAnswer), (status = 202, description = "Still running", body = Pending),
              (status = 404, description = "Not the account's WordPress", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn wp_update_themes(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<WpSlug>,
) -> Result<axum::response::Response, AppError> {
    if req
        .slug
        .as_deref()
        .is_some_and(|s| !plain(s, 1, 100, "-_."))
    {
        return Err(bad("slug", "A theme's folder name."));
    }
    run_wp(
        &state,
        &who,
        id,
        builtpanel_node_protocol::WpCommand::UpdateThemes { slug: req.slug },
        None,
    )
    .await
}

/// Update WordPress itself (to a version, or the latest).
#[utoipa::path(post, path = "/api/v1/client/apps/{id}/wordpress/core/update", tag = "apps", operation_id = "clientWordPressUpdateCore",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = WpVersion,
    responses((status = 200, description = "Done", body = WpAnswer), (status = 202, description = "Still running", body = Pending),
              (status = 404, description = "Not the account's WordPress", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn wp_update_core(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<WpVersion>,
) -> Result<axum::response::Response, AppError> {
    if req
        .version
        .as_deref()
        .is_some_and(|v| !(plain(v, 1, 20, ".") && v.starts_with(|c: char| c.is_ascii_digit())))
    {
        return Err(bad("version", "A version such as 6.8.2."));
    }
    run_wp(
        &state,
        &who,
        id,
        builtpanel_node_protocol::WpCommand::UpdateCore {
            version: req.version,
        },
        None,
    )
    .await
}

/// What hardening to apply.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WpHarden {
    /// Turn debug mode off.
    #[serde(default)]
    pub debug_off: bool,
    /// Forbid editing plugin and theme files from the admin area.
    #[serde(default)]
    pub file_edit_off: bool,
    /// Switch XML-RPC off.
    #[serde(default)]
    pub xmlrpc_off: bool,
    /// Take away write permission from everybody but the owner and group.
    #[serde(default)]
    pub fix_permissions: bool,
}

/// Check the core's files against WordPress's published checksums and look at the settings
/// that matter for safety. A changed core file is named.
#[utoipa::path(get, path = "/api/v1/client/apps/{id}/wordpress/scan", tag = "apps", operation_id = "clientWordPressScan",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id")),
    responses((status = 200, description = "The scan", body = WpAnswer), (status = 202, description = "Still running", body = Pending),
              (status = 404, description = "Not the account's WordPress", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn wp_scan(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<axum::response::Response, AppError> {
    run_wp(
        &state,
        &who,
        id,
        builtpanel_node_protocol::WpCommand::Scan,
        None,
    )
    .await
}

/// Put the safety settings right: debug off, the file editor off, XML-RPC off, permissions.
#[utoipa::path(post, path = "/api/v1/client/apps/{id}/wordpress/harden", tag = "apps", operation_id = "clientWordPressHarden",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = WpHarden,
    responses((status = 200, description = "Done", body = WpAnswer), (status = 202, description = "Still running", body = Pending),
              (status = 404, description = "Not the account's WordPress", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Nothing was asked for", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn wp_harden(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<WpHarden>,
) -> Result<axum::response::Response, AppError> {
    if !(req.debug_off || req.file_edit_off || req.xmlrpc_off || req.fix_permissions) {
        return Err(bad("debug_off", "Ask for at least one thing."));
    }
    run_wp(
        &state,
        &who,
        id,
        builtpanel_node_protocol::WpCommand::Harden {
            debug_off: req.debug_off,
            file_edit_off: req.file_edit_off,
            xmlrpc_off: req.xmlrpc_off,
            fix_permissions: req.fix_permissions,
        },
        None,
    )
    .await
}

/// Switch maintenance mode on or off.
#[utoipa::path(post, path = "/api/v1/client/apps/{id}/wordpress/maintenance", tag = "apps", operation_id = "clientWordPressMaintenance",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = WpMaintenance,
    responses((status = 200, description = "Done", body = WpAnswer), (status = 202, description = "Still running", body = Pending),
              (status = 404, description = "Not the account's WordPress", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn wp_maintenance(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<WpMaintenance>,
) -> Result<axum::response::Response, AppError> {
    run_wp(
        &state,
        &who,
        id,
        builtpanel_node_protocol::WpCommand::Maintenance { on: req.on },
        None,
    )
    .await
}

/// Set a WordPress user's password.
#[utoipa::path(post, path = "/api/v1/client/apps/{id}/wordpress/admin-password", tag = "apps", operation_id = "clientWordPressAdminPassword",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = WpNewPassword,
    responses((status = 200, description = "Done", body = WpAnswer), (status = 202, description = "Still running", body = Pending),
              (status = 404, description = "Not the account's WordPress", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A user or password that is not valid", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn wp_admin_password(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<WpNewPassword>,
) -> Result<axum::response::Response, AppError> {
    if !plain(&req.user, 1, 60, "_.-") {
        return Err(bad("user", "A WordPress login."));
    }
    if req.password.chars().count() < 12
        || req.password.len() > 128
        || req.password.chars().any(char::is_control)
    {
        return Err(bad("password", "12 to 128 characters."));
    }
    run_wp(
        &state,
        &who,
        id,
        builtpanel_node_protocol::WpCommand::ResetAdminPassword { user: req.user },
        Some(&req.password),
    )
    .await
}

/// The answer of an action that was still running.
#[utoipa::path(get, path = "/api/v1/client/apps/{id}/operations/{op}", tag = "apps", operation_id = "clientAppOperation",
    security(("session" = [])), params(("id" = Uuid, Path, description = "Application id"), ("op" = Uuid, Path, description = "Operation id")),
    responses((status = 200, description = "Its state and answer", body = WpAnswer),
              (status = 404, description = "Not the account's", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn wp_operation(
    State(state): State<AppsState>,
    who: Principal,
    Path((id, op)): Path<(Uuid, Uuid)>,
) -> Result<Json<WpAnswer>, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    store::get(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("app"))?;
    let row = operations::get(&state.db, op)
        .await
        .map_err(internal)?
        .filter(|r| r.tenant_id == Some(tenant.id) && r.kind == "run_wp_cli")
        .filter(|r| {
            r.payload.get("app_id").and_then(serde_json::Value::as_str) == Some(&id.to_string())
        })
        .ok_or_else(|| AppError::not_found("operation"))?;
    let answer = answer_of(&row);
    if answer.is_some()
        && row
            .payload
            .pointer("/command/type")
            .and_then(serde_json::Value::as_str)
            == Some("reset_admin_password")
    {
        let _ = store::forget_secret(&state.db, id, "admin_reset").await;
    }
    Ok(Json(answer.unwrap_or(WpAnswer {
        state: "running".into(),
        overview: None,
        scan: None,
        output: None,
        error: None,
    })))
}

// ------------------------------------------------------------------ cache servers

/// A cache server of the account, with what its sites need to reach it.
#[derive(Serialize, ToSchema)]
pub struct CacheView {
    /// `redis` or `memcached`.
    pub kind: String,
    /// Memory it may use (MiB).
    pub memory_mb: i32,
    /// `pending`, `ready`, `failed` or `removing`.
    pub status: String,
    /// The name the account's sites reach it by (inside the account's own network).
    pub host: Option<String>,
    /// Its port.
    pub port: Option<i32>,
    /// Redis's password (Memcached has none: only the account's own network reaches it).
    pub password: Option<String>,
    /// Why it failed.
    pub error: Option<String>,
}

impl std::fmt::Debug for CacheView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheView")
            .field("kind", &self.kind)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// The account's cache servers.
#[derive(Debug, Serialize, ToSchema)]
pub struct CacheList {
    /// `memcached` before `redis`.
    pub items: Vec<CacheView>,
    /// Memory all of them together may use (MiB; `null`: no limit).
    pub limit_mb: Option<i64>,
}

/// A cache server to make (or to change the memory of).
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetCache {
    /// `redis` or `memcached`.
    pub kind: String,
    /// Memory it may use, in MiB (64 when absent).
    #[serde(default)]
    pub memory_mb: Option<i32>,
}

fn cache_limit(plan: &plans::Plan) -> Option<i64> {
    let value = plan
        .limits
        .get("cache_memory_mb")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(64);
    (value >= 0).then_some(value)
}

fn cache_view(state: &AppsState, i: builtpanel_database::cache_instances::Instance) -> CacheView {
    let password = i.sealed_password.as_ref().and_then(|sealed| {
        state
            .master_key
            .open(
                &builtpanel_database::cache_instances::password_context(i.id),
                sealed,
            )
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    });
    CacheView {
        kind: i.kind,
        memory_mb: i.memory_mb,
        status: i.status,
        host: i.host,
        port: i.port,
        password,
        error: i.error,
    }
}

async fn cache_feature(
    state: &AppsState,
    who: &Principal,
) -> Result<(tenants::Tenant, plans::Plan), AppError> {
    let tenant = client_tenant(state, who).await?;
    let plan = plans::get(&state.db, tenant.plan_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("the plan of a tenant vanished")))?;
    if !plans::includes_feature(&state.db, plan.id, "cache")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "A cache server is not included in this plan.",
        ));
    }
    Ok((tenant, plan))
}

/// The account's cache servers and how to reach them.
#[utoipa::path(get, path = "/api/v1/client/cache", tag = "apps", operation_id = "clientListCache",
    security(("session" = [])),
    responses((status = 200, description = "The cache servers", body = CacheList),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn cache_list(
    State(state): State<AppsState>,
    who: Principal,
) -> Result<Json<CacheList>, AppError> {
    let (tenant, plan) = cache_feature(&state, &who).await?;
    let items = builtpanel_database::cache_instances::list(&state.db, tenant.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|i| cache_view(&state, i))
        .collect();
    Ok(Json(CacheList {
        items,
        limit_mb: cache_limit(&plan),
    }))
}

/// Make a cache server of your own (Redis or Memcached), or change its memory. It is
/// reachable from the account's own sites only, by the name the answer shows.
#[utoipa::path(post, path = "/api/v1/client/cache", tag = "apps", operation_id = "clientSetCache",
    security(("session" = [])), params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SetCache,
    responses((status = 202, description = "Queued", body = CacheView),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`limit_reached` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "A kind or memory that is not valid", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn cache_set(
    State(state): State<AppsState>,
    who: Principal,
    Json(req): Json<SetCache>,
) -> Result<(StatusCode, Json<CacheView>), AppError> {
    let (tenant, plan) = cache_feature(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can change its cache servers.",
        ));
    }
    if !["redis", "memcached"].contains(&req.kind.as_str()) {
        return Err(bad("kind", "redis or memcached."));
    }
    let memory = req.memory_mb.unwrap_or(64);
    if !(1..=4096).contains(&memory) {
        return Err(bad("memory_mb", "1 to 4096 MiB."));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, tenant.id)
        .await
        .map_err(internal)?;
    if let Some(limit) = cache_limit(&plan) {
        let others =
            builtpanel_database::cache_instances::memory_used(&state.db, tenant.id, &req.kind)
                .await
                .map_err(internal)?;
        if others + i64::from(memory) > limit {
            return Err(AppError::conflict(
                "limit_reached",
                format!("This plan allows {limit} MiB of cache memory in all."),
            ));
        }
    }
    let password = (req.kind == "redis").then(generate_password);
    // A new row needs its id to seal the password to it: make it, then seal.
    let id =
        builtpanel_database::cache_instances::upsert(&mut tx, tenant.id, &req.kind, memory, None)
            .await
            .map_err(internal)?;
    if let Some(password) = &password {
        let sealed = state
            .master_key
            .seal(
                &builtpanel_database::cache_instances::password_context(id),
                password.as_bytes(),
            )
            .map_err(internal)?;
        // (An existing password stays: the first one made is kept.)
        builtpanel_database::sqlx::query(
            "UPDATE cache_instances SET sealed_password = COALESCE(sealed_password, $2) WHERE id = $1",
        )
        .bind(id)
        .bind(&sealed)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    }
    let op = Operation::SetCacheInstance {
        instance_id: id,
        tenant_id: tenant.id,
        username: tenant.username.clone(),
        uid: u32::try_from(tenant.uid).unwrap_or_default(),
        kind: req.kind.clone(),
        memory_mb: u32::try_from(memory).unwrap_or(64),
        password: None::<DbPassword>,
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let queued = operations::enqueue_in(
        &mut tx,
        &NewOperation {
            node_id: tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant.id),
            payload: &payload,
            idempotency_key: &format!("cache:{}:{}", id.simple(), Uuid::now_v7().simple()),
            created_by: Some(who.account_id()),
            ttl_secs: OPERATION_TTL_SECS,
            max_attempts: OPERATION_ATTEMPTS,
        },
    )
    .await
    .map_err(|e| match e {
        EnqueueError::UnknownNode => AppError::conflict(
            "node_unavailable",
            "The node of this account no longer exists.",
        ),
        other => internal(other),
    })?;
    builtpanel_database::sqlx::query(
        "UPDATE cache_instances SET op_id = $2, status = 'pending' WHERE id = $1",
    )
    .bind(id)
    .bind(queued.operation().id)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("cache.set", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("cache_instance", id)
            .metadata(serde_json::json!({ "kind": req.kind, "memory_mb": memory })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let row = builtpanel_database::cache_instances::get(&state.db, tenant.id, &req.kind)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("cache"))?;
    Ok((StatusCode::ACCEPTED, Json(cache_view(&state, row))))
}

/// Remove a cache server (what it holds is gone).
#[utoipa::path(delete, path = "/api/v1/client/cache/{kind}", tag = "apps", operation_id = "clientRemoveCache",
    security(("session" = [])), params(("kind" = String, Path, description = "redis or memcached"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 202, description = "Queued"),
              (status = 404, description = "There is none", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn cache_remove(
    State(state): State<AppsState>,
    who: Principal,
    Path(kind): Path<String>,
) -> Result<StatusCode, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    let row = builtpanel_database::cache_instances::get(&state.db, tenant.id, &kind)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("cache"))?;
    let op = Operation::RemoveCacheInstance {
        tenant_id: tenant.id,
        username: tenant.username.clone(),
        kind: kind.clone(),
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let queued = operations::enqueue(
        &state.db,
        &NewOperation {
            node_id: tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant.id),
            payload: &payload,
            idempotency_key: &format!("cacherm:{}:{}", row.id.simple(), Uuid::now_v7().simple()),
            created_by: Some(who.account_id()),
            ttl_secs: OPERATION_TTL_SECS,
            max_attempts: OPERATION_ATTEMPTS,
        },
    )
    .await
    .map_err(|e| match e {
        EnqueueError::UnknownNode => AppError::conflict(
            "node_unavailable",
            "The node of this account no longer exists.",
        ),
        other => internal(other),
    })?;
    builtpanel_database::cache_instances::waiting_for(
        &state.db,
        row.id,
        queued.operation().id,
        "removing",
    )
    .await
    .map_err(internal)?;
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("cache.removed", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("cache_instance", row.id)
            .metadata(serde_json::json!({ "kind": kind })),
    )
    .await
    .map_err(internal)?;
    Ok(StatusCode::ACCEPTED)
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(
        client_list, client_get, client_forget, admin_list, install_wordpress, catalog_list, install_app,
        cache_list, cache_set, cache_remove, wp_overview, wp_update_plugins,
        wp_update_themes, wp_update_core, wp_maintenance, wp_admin_password, wp_operation, wp_scan,
        wp_harden, runtime::runtimes, runtime::list, runtime::create, runtime::update,
        runtime::redeploy, runtime::remove, containers::admin_list, containers::admin_add,
        containers::admin_change, containers::admin_remove, containers::client_images,
        containers::client_list, containers::client_create, containers::client_update,
        marketplace::admin_list, marketplace::admin_install, marketplace::admin_remove,
        marketplace::list, marketplace::install, marketplace::update, marketplace::uninstall,
        gitdeploy::list, gitdeploy::create,
        gitdeploy::deploy_now, gitdeploy::rotate_secret, gitdeploy::remove, gitdeploy::webhook
    ),
    components(schemas(
        AppView, AppList, InstallWordPress, CacheView, CacheList, SetCache, CatalogEntry, CatalogList, InstallApp, WpAnswer, WpSlug, WpVersion, WpMaintenance,
        runtime::RuntimeAppView, runtime::RuntimeAppList, runtime::SetRuntimeApp,
        runtime::Runtimes, runtime::Redeploy, containers::ImageView, containers::SetImage,
        containers::ContainerView, containers::CreateContainer, containers::ChangeContainer, gitdeploy::GitDeployView, gitdeploy::GitDeployList,
        gitdeploy::CreateGitDeploy, gitdeploy::WebhookAnswer,
        marketplace::AdminSoftwareView, marketplace::SoftwareView, marketplace::Installed, marketplace::InstallRequest,
        WpNewPassword, Pending, WpHarden, builtpanel_node_protocol::WpOverview,
        builtpanel_node_protocol::WpItem, builtpanel_node_protocol::WpScan
    )),
    tags((name = "apps", description = "Applications on domains and paths"))
)]
pub struct AppsApi;
