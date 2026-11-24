//! Applications a client runs themselves: Node.js and Python (T12.07, T12.08).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/client/runtime-apps` | the owner; the `runtime_apps` feature |
//! | GET | `/api/v1/client/runtime-apps/runtimes` | the owner |
//! | PUT, DELETE | `/api/v1/client/runtime-apps/{id}` | the owner |
//! | POST | `/api/v1/client/runtime-apps/{id}/redeploy` | the owner |
//!
//! An application is a container of the account (see the node agent's `runtimeapp`) that runs
//! the account's files from a folder with a start command, behind one of the account's domains:
//! once it runs, the web server passes that domain on to it. The environment holds secrets, so it
//! is stored sealed and written into the operation only as it is sent.

use std::collections::BTreeMap;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    operations::{self, EnqueueError, NewOperation},
    plans, runtime_apps as store,
    sqlx::PgConnection,
    tenants,
};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_node_protocol::Operation;
use builtpanel_rbac::Principal;
use builtpanel_tenant_stack::{Allowlist, Role};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{AppsState, OPERATION_ATTEMPTS, OPERATION_TTL_SECS, bad, client_tenant, internal};

/// Most applications an account has when its plan does not say.
pub(crate) const DEFAULT_MAX_APPS: i64 = 3;
/// Memory all of an account's applications may use together, when its plan does not say (MiB).
pub(crate) const DEFAULT_MEMORY_MB: i64 = 1024;

/// Routes with full paths; merged into the apps module's client router.
pub fn router() -> Router<AppsState> {
    Router::new()
        .route("/api/v1/client/runtime-apps", get(list).post(create))
        .route("/api/v1/client/runtime-apps/runtimes", get(runtimes))
        .route(
            "/api/v1/client/runtime-apps/{id}",
            axum::routing::put(update).delete(remove),
        )
        .route("/api/v1/client/runtime-apps/{id}/redeploy", post(redeploy))
}

/// An application of the account.
#[derive(Debug, Serialize, ToSchema)]
pub struct RuntimeAppView {
    /// Id.
    pub id: Uuid,
    /// Short name.
    pub name: String,
    /// `node` or `python`.
    pub kind: String,
    /// The runtime's version.
    pub version: String,
    /// The folder below the web directory.
    pub dir: String,
    /// What starts it.
    pub start_command: String,
    /// Whether dependencies are installed before it starts.
    pub install: bool,
    /// The port it listens on (given to it as `PORT`).
    pub port: i32,
    /// Memory limit, MiB.
    pub memory_mb: i32,
    /// The names of its environment variables (the values are never shown).
    pub env_names: Vec<String>,
    /// The domain that shows it.
    pub domain_id: Option<Uuid>,
    /// That domain's name.
    pub domain: Option<String>,
    /// `pending`, `ready`, `failed` or `removing`.
    pub status: String,
    /// Why it failed.
    pub error: Option<String>,
}

/// The account's applications.
#[derive(Debug, Serialize, ToSchema)]
pub struct RuntimeAppList {
    /// The applications.
    pub items: Vec<RuntimeAppView>,
    /// How many the plan allows; `None` when unlimited.
    pub limit: Option<i64>,
}

pub(crate) async fn view_all(
    state: &AppsState,
    rows: Vec<store::RuntimeApp>,
) -> Result<Vec<RuntimeAppView>, AppError> {
    let names: BTreeMap<Uuid, String> = builtpanel_database::sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, fqdn FROM domains WHERE id = ANY($1)",
    )
    .bind(rows.iter().filter_map(|r| r.domain_id).collect::<Vec<_>>())
    .fetch_all(&state.db)
    .await
    .map_err(internal)?
    .into_iter()
    .collect();
    Ok(rows
        .into_iter()
        .map(|r| RuntimeAppView {
            domain: r.domain_id.and_then(|d| names.get(&d).cloned()),
            id: r.id,
            name: r.name,
            kind: r.kind,
            version: r.version,
            dir: r.dir,
            start_command: r.start_command,
            install: r.install,
            port: r.port,
            memory_mb: r.memory_mb,
            env_names: r.env_names,
            domain_id: r.domain_id,
            status: r.status,
            error: r.error,
        })
        .collect())
}

pub(crate) fn limit_of(plan: &plans::Plan, key: &str, default: i64) -> Option<i64> {
    let value = plan
        .limits
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(default);
    (value >= 0).then_some(value)
}

pub(crate) async fn feature(
    state: &AppsState,
    who: &Principal,
) -> Result<(tenants::Tenant, plans::Plan), AppError> {
    let tenant = client_tenant(state, who).await?;
    let plan = plans::get(&state.db, tenant.plan_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("the plan of a tenant vanished")))?;
    if !plans::includes_feature(&state.db, plan.id, "runtime_apps")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Running your own applications is not included in this plan.",
        ));
    }
    Ok((tenant, plan))
}

/// The words of a start command. Quotes (`'…'`, `"…"`) group words and `\` escapes the next
/// character; `$PORT` and `${PORT}` become the port. Nothing else is interpreted: no
/// redirection, no `;`, no substitution (the command is never run through a shell).
///
/// # Errors
/// What is wrong with it.
pub fn split_command(text: &str, port: i32) -> Result<Vec<String>, &'static str> {
    let text = text
        .replace("${PORT}", &port.to_string())
        .replace("$PORT", &port.to_string());
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (Some(q), c) if c == q => quote = None,
            (Some('\''), c) => cur.push(c),
            (_, '\\') => {
                cur.push(chars.next().ok_or("a backslash at the end")?);
                in_word = true;
            }
            (_, c) => {
                if c.is_control() {
                    return Err("a control character in the command");
                }
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("a quote is not closed");
    }
    if in_word {
        words.push(cur);
    }
    if words.is_empty() {
        return Err("the command is empty");
    }
    if words.len() > 32 || words.iter().any(|w| w.len() > 500) {
        return Err("at most 32 words of 500 characters");
    }
    Ok(words)
}

/// The settings a person gives for an application.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetRuntimeApp {
    /// Short name (2-32 of a-z, 0-9, `-`); needed when making one, not changeable.
    pub name: Option<String>,
    /// `node` or `python`; needed when making one, not changeable.
    pub kind: Option<String>,
    /// The runtime's version (see `/runtimes`); the newest when not given.
    pub version: Option<String>,
    /// The folder of the application below the web directory (empty: the directory itself).
    pub dir: Option<String>,
    /// What starts it, for example `node server.js` or
    /// `python -m gunicorn app:app -b 0.0.0.0:$PORT`.
    pub start_command: String,
    /// Install the dependencies first (default: yes).
    pub install: Option<bool>,
    /// The port it listens on (default 3000).
    pub port: Option<i32>,
    /// Memory limit in MiB (default 256).
    pub memory_mb: Option<i32>,
    /// The environment, name to value. When given it replaces the whole environment; when left
    /// out the environment stays as it is.
    pub env: Option<BTreeMap<String, String>>,
    /// The domain that shows the application (one of the account's; one application each).
    pub domain_id: Option<Uuid>,
}

pub(crate) fn check_env(env: &BTreeMap<String, String>) -> Result<(), AppError> {
    if env.len() > 50 {
        return Err(bad("env", "At most 50 variables."));
    }
    for (k, v) in env {
        let ok = !k.is_empty()
            && k.len() <= 64
            && k.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
            && k.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
        let reserved = [
            "PATH",
            "HOME",
            "PORT",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "LD_AUDIT",
            "PYTHONPATH",
            "PYTHONHOME",
        ]
        .contains(&k.as_str());
        if !ok || reserved {
            return Err(bad(
                "env",
                "Names are capitals, digits and _; PATH, HOME, PORT, LD_* and PYTHONPATH are the panel's.",
            ));
        }
        if v.len() > 4096 || v.contains('\0') {
            return Err(bad("env", "A value is too long (4096 bytes at most)."));
        }
    }
    Ok(())
}

fn valid_dir(dir: &str) -> bool {
    dir.len() <= 200
        && !dir.starts_with('/')
        && !dir.split('/').any(|p| p == ".." || p == ".")
        && !dir.contains("//")
        && dir
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
}

pub(crate) fn valid_name(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// What the account's applications may be, per kind.
#[derive(Debug, Serialize, ToSchema)]
pub struct Runtimes {
    /// Node.js versions, the newest first.
    pub node: Vec<String>,
    /// Python versions, the newest first.
    pub python: Vec<String>,
}

/// The versions of Node.js and Python that can be chosen.
#[utoipa::path(get, path = "/api/v1/client/runtime-apps/runtimes", tag = "apps",
    operation_id = "clientRuntimeVersions", security(("session" = [])),
    responses((status = 200, description = "The versions", body = Runtimes),
              (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn runtimes(
    State(state): State<AppsState>,
    who: Principal,
) -> Result<Json<Runtimes>, AppError> {
    client_tenant(&state, &who).await?;
    let list = Allowlist::embedded().map_err(internal)?;
    let own = |role| {
        list.runtime_versions(role)
            .into_iter()
            .map(str::to_owned)
            .collect()
    };
    Ok(Json(Runtimes {
        node: own(Role::Node),
        python: own(Role::Python),
    }))
}

/// The account's applications.
#[utoipa::path(get, path = "/api/v1/client/runtime-apps", tag = "apps",
    operation_id = "clientListRuntimeApps", security(("session" = [])),
    responses((status = 200, description = "The applications", body = RuntimeAppList),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn list(
    State(state): State<AppsState>,
    who: Principal,
) -> Result<Json<RuntimeAppList>, AppError> {
    let (tenant, plan) = feature(&state, &who).await?;
    let rows = store::list(&state.db, tenant.id).await.map_err(internal)?;
    Ok(Json(RuntimeAppList {
        items: view_all(&state, rows).await?,
        limit: limit_of(&plan, "max_runtime_apps", DEFAULT_MAX_APPS),
    }))
}

/// Queues the node's work for an application, inside the caller's transaction.
pub(crate) async fn queue_set(
    conn: &mut PgConnection,
    who: &Principal,
    tenant: &tenants::Tenant,
    app: &store::RuntimeApp,
    install: bool,
) -> Result<Uuid, AppError> {
    // A container runs an approved image as it is: no command, no install, no files.
    let image = if app.kind == "container" {
        Some(
            builtpanel_database::container_images::of_app(conn, app.id)
                .await
                .map_err(internal)?
                .ok_or_else(|| bad("image_id", "The image of this container is gone."))?
                .reference,
        )
    } else {
        None
    };
    let command = if image.is_some() {
        Vec::new()
    } else {
        split_command(&app.start_command, app.port).map_err(|why| bad("start_command", why))?
    };
    let op = Operation::SetRuntimeApp {
        app_id: app.id,
        tenant_id: tenant.id,
        username: tenant.username.clone(),
        uid: u32::try_from(tenant.uid).unwrap_or_default(),
        kind: app.kind.clone(),
        version: app.version.clone(),
        name: app.name.clone(),
        dir: app.dir.clone(),
        command,
        install,
        port: u16::try_from(app.port).unwrap_or(3000),
        memory_mb: u32::try_from(app.memory_mb).unwrap_or(256),
        env: None,
        image,
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let queued = operations::enqueue_in(
        conn,
        &NewOperation {
            node_id: tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant.id),
            payload: &payload,
            idempotency_key: &format!("runtimeapp:{}:{}", app.id.simple(), Uuid::now_v7().simple()),
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
    let op_id = queued.operation().id;
    store::waiting_for(conn, app.id, op_id, "pending")
        .await
        .map_err(internal)?;
    Ok(op_id)
}

struct Checked {
    name: String,
    kind: String,
    version: String,
    dir: String,
    port: i32,
    memory_mb: i32,
    install: bool,
    env_names: Vec<String>,
}

fn check(req: &SetRuntimeApp, existing: Option<&store::RuntimeApp>) -> Result<Checked, AppError> {
    let (name, kind) = match existing {
        Some(e) => (e.name.clone(), e.kind.clone()),
        None => {
            let name = req.name.clone().unwrap_or_default();
            if !valid_name(&name) {
                return Err(bad(
                    "name",
                    "2 to 32 of a-z, 0-9 and -, starting with a letter.",
                ));
            }
            let kind = req.kind.clone().unwrap_or_default();
            if !["node", "python"].contains(&kind.as_str()) {
                return Err(bad("kind", "node or python."));
            }
            (name, kind)
        }
    };
    let role = if kind == "node" {
        Role::Node
    } else {
        Role::Python
    };
    let list = Allowlist::embedded().map_err(internal)?;
    let versions = list.runtime_versions(role);
    let version = match &req.version {
        Some(v) if versions.contains(&v.as_str()) => v.clone(),
        Some(_) => {
            return Err(bad(
                "version",
                "This version is not available (see /runtimes).",
            ));
        }
        None => existing
            .map(|e| e.version.clone())
            .or_else(|| versions.first().map(|v| (*v).to_owned()))
            .ok_or_else(|| internal(std::io::Error::other("no runtime image")))?,
    };
    let dir = req
        .dir
        .clone()
        .or_else(|| existing.map(|e| e.dir.clone()))
        .unwrap_or_default();
    if !valid_dir(&dir) {
        return Err(bad(
            "dir",
            "A folder below the web directory, without .. or odd characters.",
        ));
    }
    let port = req
        .port
        .or_else(|| existing.map(|e| e.port))
        .unwrap_or(3000);
    if !(1024..=65_535).contains(&port) {
        return Err(bad("port", "1024 to 65535."));
    }
    let memory_mb = req
        .memory_mb
        .or_else(|| existing.map(|e| e.memory_mb))
        .unwrap_or(256);
    if !(32..=4096).contains(&memory_mb) {
        return Err(bad("memory_mb", "32 to 4096 MiB."));
    }
    split_command(&req.start_command, port).map_err(|why| bad("start_command", why))?;
    if let Some(env) = &req.env {
        check_env(env)?;
    }
    Ok(Checked {
        name,
        kind,
        version,
        dir,
        port,
        memory_mb,
        install: req
            .install
            .or_else(|| existing.map(|e| e.install))
            .unwrap_or(true),
        env_names: match (&req.env, existing) {
            (Some(env), _) => env.keys().cloned().collect(),
            (None, Some(e)) => e.env_names.clone(),
            (None, None) => Vec::new(),
        },
    })
}

pub(crate) fn write_error(e: store::WriteError) -> AppError {
    match e {
        store::WriteError::Taken => AppError::conflict(
            "app_taken",
            "An application with this name exists, or this domain already has one.",
        ),
        store::WriteError::UnknownDomain => bad("domain_id", "This is not one of your domains."),
        store::WriteError::Database(db) => internal(db),
    }
}

/// Seals an environment to an application.
pub(crate) fn seal(
    state: &AppsState,
    id: Uuid,
    env: &BTreeMap<String, String>,
) -> Result<Vec<u8>, AppError> {
    let json = serde_json::to_vec(env).map_err(internal)?;
    state
        .master_key
        .seal(&store::env_context(id), &json)
        .map_err(internal)
}

/// Make an application: a container of your own that runs your files with a start command and,
/// when a domain is given, shows that domain. The answer is `pending` until the node has done it.
#[utoipa::path(post, path = "/api/v1/client/runtime-apps", tag = "apps",
    operation_id = "clientCreateRuntimeApp", security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SetRuntimeApp,
    responses((status = 202, description = "Queued", body = RuntimeAppView),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`limit_reached`, `app_taken` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Invalid input", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn create(
    State(state): State<AppsState>,
    who: Principal,
    Json(req): Json<SetRuntimeApp>,
) -> Result<(StatusCode, Json<RuntimeAppView>), AppError> {
    let (tenant, plan) = feature(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can change its applications.",
        ));
    }
    let c = check(&req, None)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, tenant.id)
        .await
        .map_err(internal)?;
    if let Some(limit) = limit_of(&plan, "max_runtime_apps", DEFAULT_MAX_APPS)
        && store::count(&mut tx, tenant.id).await.map_err(internal)? >= limit
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("This plan allows {limit} application(s)."),
        ));
    }
    if let Some(limit) = limit_of(&plan, "runtime_memory_mb", DEFAULT_MEMORY_MB)
        && store::memory_used(&mut tx, tenant.id, None)
            .await
            .map_err(internal)?
            + i64::from(c.memory_mb)
            > limit
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("This plan allows {limit} MiB of memory for applications in all."),
        ));
    }
    let id = store::insert(
        &mut tx,
        tenant.id,
        &store::Write {
            name: &c.name,
            kind: &c.kind,
            version: &c.version,
            dir: &c.dir,
            start_command: &req.start_command,
            install: c.install,
            port: c.port,
            memory_mb: c.memory_mb,
            domain_id: req.domain_id,
            env_names: &c.env_names,
        },
    )
    .await
    .map_err(write_error)?;
    let sealed = seal(&state, id, &req.env.clone().unwrap_or_default())?;
    store::set_env(&mut tx, id, &sealed)
        .await
        .map_err(internal)?;
    let row = load_in(&mut tx, tenant.id, id).await?;
    queue_set(&mut tx, &who, &tenant, &row, c.install).await?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("runtime_app.created", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("runtime_app", id)
            .metadata(serde_json::json!({ "name": c.name, "kind": c.kind, "version": c.version })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    answer(&state, &tenant, id, StatusCode::ACCEPTED).await
}

pub(crate) async fn load_in(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    id: Uuid,
) -> Result<store::RuntimeApp, AppError> {
    store::get_in(conn, tenant_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("application"))
}

pub(crate) async fn answer(
    state: &AppsState,
    tenant: &tenants::Tenant,
    id: Uuid,
    status: StatusCode,
) -> Result<(StatusCode, Json<RuntimeAppView>), AppError> {
    let row = store::get(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("application"))?;
    let mut views = view_all(state, vec![row]).await?;
    Ok((status, Json(views.remove(0))))
}

/// Change an application's settings (and run it again with them).
#[utoipa::path(put, path = "/api/v1/client/runtime-apps/{id}", tag = "apps",
    operation_id = "clientUpdateRuntimeApp", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Application id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SetRuntimeApp,
    responses((status = 202, description = "Queued", body = RuntimeAppView),
              (status = 404, description = "No such application", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`limit_reached`, `app_taken` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Invalid input", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn update(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<SetRuntimeApp>,
) -> Result<(StatusCode, Json<RuntimeAppView>), AppError> {
    let (tenant, plan) = feature(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can change its applications.",
        ));
    }
    let existing = store::get(&state.db, tenant.id, id)
        .await
        .map_err(internal)?
        .filter(|a| a.status != "removing")
        .ok_or_else(|| AppError::not_found("application"))?;
    if existing.kind == "container" {
        return Err(bad(
            "kind",
            "A container is changed through /api/v1/client/container-apps/{id}.",
        ));
    }
    if req.name.as_deref().is_some_and(|n| n != existing.name)
        || req.kind.as_deref().is_some_and(|k| k != existing.kind)
    {
        return Err(bad("name", "The name and the kind cannot be changed."));
    }
    let c = check(&req, Some(&existing))?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, tenant.id)
        .await
        .map_err(internal)?;
    if let Some(limit) = limit_of(&plan, "runtime_memory_mb", DEFAULT_MEMORY_MB)
        && store::memory_used(&mut tx, tenant.id, Some(id))
            .await
            .map_err(internal)?
            + i64::from(c.memory_mb)
            > limit
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("This plan allows {limit} MiB of memory for applications in all."),
        ));
    }
    let domain_id = match req.domain_id {
        Some(d) => Some(d),
        None => existing.domain_id,
    };
    store::update(
        &mut tx,
        tenant.id,
        id,
        &store::Write {
            name: &c.name,
            kind: &c.kind,
            version: &c.version,
            dir: &c.dir,
            start_command: &req.start_command,
            install: c.install,
            port: c.port,
            memory_mb: c.memory_mb,
            domain_id,
            env_names: &c.env_names,
        },
    )
    .await
    .map_err(write_error)?;
    if let Some(env) = &req.env {
        let sealed = seal(&state, id, env)?;
        store::set_env(&mut tx, id, &sealed)
            .await
            .map_err(internal)?;
    }
    let row = load_in(&mut tx, tenant.id, id).await?;
    queue_set(&mut tx, &who, &tenant, &row, c.install).await?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("runtime_app.updated", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("runtime_app", id)
            .metadata(serde_json::json!({ "name": c.name })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    answer(&state, &tenant, id, StatusCode::ACCEPTED).await
}

/// Whether to install the dependencies again when redeploying.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Redeploy {
    /// Install the dependencies first (default: yes).
    pub install: Option<bool>,
}

/// Run the application again from its files (after you changed them): the dependencies are
/// installed again unless `install` is false.
#[utoipa::path(post, path = "/api/v1/client/runtime-apps/{id}/redeploy", tag = "apps",
    operation_id = "clientRedeployRuntimeApp", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Application id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body(content = Redeploy, description = "Optional"),
    responses((status = 202, description = "Queued", body = RuntimeAppView),
              (status = 404, description = "No such application", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn redeploy(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
    body: Option<Json<Redeploy>>,
) -> Result<(StatusCode, Json<RuntimeAppView>), AppError> {
    let (tenant, _plan) = feature(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can change its applications.",
        ));
    }
    let install = body.and_then(|b| b.0.install).unwrap_or(true);
    let mut tx = state.db.begin().await.map_err(internal)?;
    let row = load_in(&mut tx, tenant.id, id).await?;
    if row.status == "removing" {
        return Err(AppError::not_found("application"));
    }
    queue_set(&mut tx, &who, &tenant, &row, install).await?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("runtime_app.redeployed", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("runtime_app", id),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    answer(&state, &tenant, id, StatusCode::ACCEPTED).await
}

/// Remove an application (its container goes; your files stay).
#[utoipa::path(delete, path = "/api/v1/client/runtime-apps/{id}", tag = "apps",
    operation_id = "clientRemoveRuntimeApp", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Application id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 202, description = "Queued"),
              (status = 404, description = "No such application", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn remove(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let row = load_in(&mut tx, tenant.id, id).await?;
    let op = Operation::RemoveRuntimeApp {
        tenant_id: tenant.id,
        username: tenant.username.clone(),
        name: row.name.clone(),
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let queued = operations::enqueue_in(
        &mut tx,
        &NewOperation {
            node_id: tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant.id),
            payload: &payload,
            idempotency_key: &format!("runtimeappdel:{}:{}", id.simple(), Uuid::now_v7().simple()),
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
    store::waiting_for(&mut tx, id, queued.operation().id, "removing")
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("runtime_app.removed", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("runtime_app", id)
            .metadata(serde_json::json!({ "name": row.name })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_split_into_words_and_never_interpreted() {
        let words = |t: &str| split_command(t, 3000).unwrap();
        assert_eq!(words("node server.js"), ["node", "server.js"]);
        assert_eq!(
            words("python -m gunicorn app:app -b 0.0.0.0:$PORT"),
            ["python", "-m", "gunicorn", "app:app", "-b", "0.0.0.0:3000"]
        );
        assert_eq!(
            words("node -e 'console.log(\"a b\")'"),
            ["node", "-e", "console.log(\"a b\")"]
        );
        assert_eq!(
            words("sh -c \"echo $HOME; id\""),
            ["sh", "-c", "echo $HOME; id"]
        );
        assert_eq!(words("a\\ b c"), ["a b", "c"]);
        // `;`, `|` and `$(…)` are only characters of a word: nothing runs them.
        assert_eq!(
            words("node a.js ; rm -rf /"),
            ["node", "a.js", ";", "rm", "-rf", "/"]
        );
        for bad in ["", "   ", "node 'unclosed", "a\\", "node\u{0}x"] {
            assert!(split_command(bad, 3000).is_err(), "{bad:?}");
        }
        assert!(split_command(&"a ".repeat(40), 3000).is_err());
    }

    #[test]
    fn names_folders_and_environment_names_are_checked() {
        assert!(valid_name("api") && valid_name("my-app-2"));
        assert!(
            !valid_name("a") && !valid_name("-api") && !valid_name("api-") && !valid_name("Api")
        );
        assert!(valid_dir("") && valid_dir("apps/api") && !valid_dir("../x") && !valid_dir("/etc"));
        let mut env = BTreeMap::new();
        env.insert("DATABASE_URL".to_owned(), "x".to_owned());
        assert!(check_env(&env).is_ok());
        for bad in ["PATH", "port", "A B", "LD_PRELOAD", ""] {
            let mut env = BTreeMap::new();
            env.insert(bad.to_owned(), "x".to_owned());
            assert!(check_env(&env).is_err(), "{bad}");
        }
    }
}
