//! Deploying from a git repository (T12.09).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/client/git-deploys` | the owner; the `git_deploy` feature |
//! | POST | `/api/v1/client/git-deploys/{id}/deploy` | the owner |
//! | POST | `/api/v1/client/git-deploys/{id}/webhook-secret` | the owner |
//! | DELETE | `/api/v1/client/git-deploys/{id}` | the owner |
//! | POST | `/api/v1/public/git-webhook/{id}` | the webhook's signature |
//!
//! A deploy fetches one branch of a repository (https), builds it in a sandbox and switches the
//! account's folder `dest` to the new release with one rename (see the node agent's
//! `gitdeploy`). A push can start it: the repository's host calls the webhook, signed with the
//! deploy's secret (GitHub's `X-Hub-Signature-256`, or GitLab's `X-Gitlab-Token`). The secret is
//! shown once, when it is made.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    git_deploys as store,
    operations::{self, EnqueueError, NewOperation},
    plans,
    sqlx::PgConnection,
    tenants,
};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_node_protocol::{DbPassword, Operation};
use builtpanel_rbac::Principal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    AppsState, OPERATION_ATTEMPTS, OPERATION_TTL_SECS, bad, client_tenant, internal,
    runtime::split_command,
};

/// Most deploys an account has when its plan does not say.
const DEFAULT_MAX: i64 = 5;

/// The routes that need a session.
pub fn router() -> Router<AppsState> {
    Router::new()
        .route("/api/v1/client/git-deploys", get(list).post(create))
        .route(
            "/api/v1/client/git-deploys/{id}",
            axum::routing::delete(remove),
        )
        .route("/api/v1/client/git-deploys/{id}/deploy", post(deploy_now))
        .route(
            "/api/v1/client/git-deploys/{id}/webhook-secret",
            post(rotate_secret),
        )
}

/// The route the repository's host calls (no session; the signature is the proof).
pub fn public_router() -> Router<AppsState> {
    Router::new().route("/api/v1/public/git-webhook/{id}", post(webhook))
}

/// A deploy of the account.
#[derive(Debug, Serialize, ToSchema)]
pub struct GitDeployView {
    /// Id.
    pub id: Uuid,
    /// Short name.
    pub name: String,
    /// The repository.
    pub repo_url: String,
    /// The branch.
    pub branch: String,
    /// The folder (below the web directory) that links to the release in use.
    pub dest: String,
    /// The build steps.
    pub build_steps: Vec<String>,
    /// The runtime of the build.
    pub build_kind: Option<String>,
    /// The runtime's version.
    pub build_version: Option<String>,
    /// The application restarted after a deploy.
    pub app_id: Option<Uuid>,
    /// Whether a token for a private repository is kept.
    pub has_token: bool,
    /// `idle`, `deploying`, `ok`, `failed` or `removing`.
    pub status: String,
    /// The commit that is live.
    pub last_commit: Option<String>,
    /// The end of the last deploy's log.
    pub last_log: Option<String>,
    /// Why the last deploy failed.
    pub last_error: Option<String>,
    /// When the last deploy went live (Unix seconds).
    pub last_deployed_at: Option<i64>,
    /// The address the repository's host calls on a push.
    pub webhook_path: String,
    /// The webhook's secret: only in the answer that made it (or made a new one).
    pub webhook_secret: Option<String>,
}

fn view(d: store::GitDeploy, secret: Option<String>) -> GitDeployView {
    GitDeployView {
        webhook_path: format!("/api/v1/public/git-webhook/{}", d.id),
        has_token: d.sealed_token.is_some(),
        id: d.id,
        name: d.name,
        repo_url: d.repo_url,
        branch: d.branch,
        dest: d.dest,
        build_steps: d.build_steps,
        build_kind: d.build_kind,
        build_version: d.build_version,
        app_id: d.app_id,
        status: d.status,
        last_commit: d.last_commit,
        last_log: d.last_log,
        last_error: d.last_error,
        last_deployed_at: d.last_deployed_at,
        webhook_secret: secret,
    }
}

/// The account's deploys.
#[derive(Debug, Serialize, ToSchema)]
pub struct GitDeployList {
    /// The deploys.
    pub items: Vec<GitDeployView>,
    /// How many the plan allows; `None` when unlimited.
    pub limit: Option<i64>,
}

fn limit_of(plan: &plans::Plan) -> Option<i64> {
    let v = plan
        .limits
        .get("max_git_deploys")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(DEFAULT_MAX);
    (v >= 0).then_some(v)
}

async fn feature(
    state: &AppsState,
    who: &Principal,
) -> Result<(tenants::Tenant, plans::Plan), AppError> {
    let tenant = client_tenant(state, who).await?;
    let plan = plans::get(&state.db, tenant.plan_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| internal(std::io::Error::other("the plan of a tenant vanished")))?;
    if !plans::includes_feature(&state.db, plan.id, "git_deploy")
        .await
        .map_err(internal)?
    {
        return Err(AppError::forbidden(
            "feature_disabled",
            "Deploying from git is not included in this plan.",
        ));
    }
    Ok((tenant, plan))
}

/// The account's deploys.
#[utoipa::path(get, path = "/api/v1/client/git-deploys", tag = "apps",
    operation_id = "clientListGitDeploys", security(("session" = [])),
    responses((status = 200, description = "The deploys", body = GitDeployList),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn list(
    State(state): State<AppsState>,
    who: Principal,
) -> Result<Json<GitDeployList>, AppError> {
    let (tenant, plan) = feature(&state, &who).await?;
    let items = store::list(&state.db, tenant.id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|d| view(d, None))
        .collect();
    Ok(Json(GitDeployList {
        items,
        limit: limit_of(&plan),
    }))
}

/// A deploy to make.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateGitDeploy {
    /// Short name (2-32 of a-z, 0-9, `-`).
    pub name: String,
    /// The repository: `https://…`, without a login.
    pub repo_url: String,
    /// The branch (default `main`).
    pub branch: Option<String>,
    /// The folder below the web directory that becomes the link to the release in use (it must
    /// not exist as a real folder yet).
    pub dest: String,
    /// Commands that build the release, one line each, words without a shell
    /// (for example `npm ci`, `npm run build`). At most 5.
    pub build_steps: Option<Vec<String>>,
    /// `node` or `python`; needed when there are build steps.
    pub build_kind: Option<String>,
    /// The runtime's version (the newest when not given).
    pub build_version: Option<String>,
    /// One of the account's applications (see `/runtime-apps`) to restart after each deploy.
    pub app_id: Option<Uuid>,
    /// A token for a private repository (`token`, or `user:token`).
    pub token: Option<String>,
    /// Deploy at once (default: yes).
    pub deploy_now: Option<bool>,
}

fn valid_name(name: &str) -> bool {
    (2..=32).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_dest(d: &str) -> bool {
    !d.is_empty()
        && d.len() <= 200
        && !d.starts_with('/')
        && !d
            .split('/')
            .any(|p| p.is_empty() || p == ".." || p == "." || p.starts_with(".bp-"))
        && d.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
}

fn valid_branch(b: &str) -> bool {
    !b.is_empty()
        && b.len() <= 100
        && !b.starts_with('-')
        && !b.split('/').any(|p| p == ".." || p == ".")
        && b.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'/' | b'-'))
}

/// An https address without a login or white space.
fn valid_repo(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split('/').next().unwrap_or_default();
    (12..=300).contains(&url.len())
        && !host.is_empty()
        && !host.contains('@')
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn webhook_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// The application's name, for the node.
async fn app_name(
    conn: &mut PgConnection,
    tenant_id: Uuid,
    app_id: Uuid,
) -> Result<Option<String>, AppError> {
    Ok(
        builtpanel_database::runtime_apps::get_in(conn, tenant_id, app_id)
            .await
            .map_err(internal)?
            .map(|a| a.name),
    )
}

async fn queue_deploy(
    conn: &mut PgConnection,
    created_by: Option<Uuid>,
    tenant: &tenants::Tenant,
    d: &store::GitDeploy,
) -> Result<Uuid, AppError> {
    let build_steps = d
        .build_steps
        .iter()
        .map(|line| split_command(line, 3000).map_err(|why| bad("build_steps", why)))
        .collect::<Result<Vec<_>, _>>()?;
    let restart_app = match d.app_id {
        Some(app) => app_name(conn, tenant.id, app).await?,
        None => None,
    };
    let op = Operation::RunGitDeploy {
        deploy_id: d.id,
        tenant_id: tenant.id,
        username: tenant.username.clone(),
        uid: u32::try_from(tenant.uid).unwrap_or_default(),
        name: d.name.clone(),
        repo_url: d.repo_url.clone(),
        branch: d.branch.clone(),
        dest: d.dest.clone(),
        build_kind: d.build_kind.clone(),
        build_version: d.build_version.clone(),
        build_steps,
        restart_app,
        token: None::<DbPassword>,
    };
    let payload = serde_json::to_value(&op).map_err(internal)?;
    let queued = operations::enqueue_in(
        conn,
        &NewOperation {
            node_id: tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant.id),
            payload: &payload,
            idempotency_key: &format!("gitdeploy:{}:{}", d.id.simple(), Uuid::now_v7().simple()),
            created_by,
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
    store::waiting_for(conn, d.id, op_id, "deploying")
        .await
        .map_err(internal)?;
    Ok(op_id)
}

/// Make a deploy: the repository, the branch, how it is built and where it goes. The answer
/// has the webhook's secret, shown only this once.
#[utoipa::path(post, path = "/api/v1/client/git-deploys", tag = "apps",
    operation_id = "clientCreateGitDeploy", security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = CreateGitDeploy,
    responses((status = 202, description = "Made (and queued unless `deploy_now` is false)", body = GitDeployView),
              (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`limit_reached`, `deploy_taken` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 422, description = "Invalid input", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn create(
    State(state): State<AppsState>,
    who: Principal,
    Json(req): Json<CreateGitDeploy>,
) -> Result<(StatusCode, Json<GitDeployView>), AppError> {
    let (tenant, plan) = feature(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can change its deploys.",
        ));
    }
    if !valid_name(&req.name) {
        return Err(bad(
            "name",
            "2 to 32 of a-z, 0-9 and -, starting with a letter.",
        ));
    }
    if !valid_repo(&req.repo_url) {
        return Err(bad("repo_url", "An https address without a login."));
    }
    let branch = req.branch.clone().unwrap_or_else(|| "main".to_owned());
    if !valid_branch(&branch) {
        return Err(bad("branch", "A branch name."));
    }
    if !valid_dest(&req.dest) {
        return Err(bad(
            "dest",
            "A folder below the web directory, without .. or odd characters.",
        ));
    }
    let steps = req.build_steps.clone().unwrap_or_default();
    if steps.len() > 5 {
        return Err(bad("build_steps", "At most 5."));
    }
    for line in &steps {
        split_command(line, 3000).map_err(|why| bad("build_steps", why))?;
    }
    let (kind, version) = if steps.is_empty() {
        (None, None)
    } else {
        let kind = req.build_kind.clone().unwrap_or_default();
        let role = match kind.as_str() {
            "node" => builtpanel_tenant_stack::Role::Node,
            "python" => builtpanel_tenant_stack::Role::Python,
            _ => return Err(bad("build_kind", "node or python.")),
        };
        let list = builtpanel_tenant_stack::Allowlist::embedded().map_err(internal)?;
        let versions = list.runtime_versions(role);
        let version = match &req.build_version {
            Some(v) if versions.contains(&v.as_str()) => v.clone(),
            Some(_) => return Err(bad("build_version", "This version is not available.")),
            None => versions
                .first()
                .map(|v| (*v).to_owned())
                .ok_or_else(|| internal(std::io::Error::other("no runtime image")))?,
        };
        (Some(kind), Some(version))
    };
    if let Some(token) = &req.token
        && (token.is_empty()
            || token.len() > 300
            || token.chars().any(|c| c.is_whitespace() || c.is_control()))
    {
        return Err(bad("token", "No spaces, at most 300 characters."));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, tenant.id)
        .await
        .map_err(internal)?;
    if let Some(limit) = limit_of(&plan)
        && store::count(&mut tx, tenant.id).await.map_err(internal)? >= limit
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("This plan allows {limit} deploy(s)."),
        ));
    }
    let secret = webhook_secret();
    let id = Uuid::now_v7();
    let sealed = state
        .master_key
        .seal(&store::webhook_context(id), secret.as_bytes())
        .map_err(internal)?;
    store::insert(
        &mut tx,
        id,
        tenant.id,
        &store::Write {
            name: &req.name,
            repo_url: &req.repo_url,
            branch: &branch,
            dest: &req.dest,
            build_steps: &steps,
            build_kind: kind.as_deref(),
            build_version: version.as_deref(),
            app_id: req.app_id,
        },
        &sealed,
    )
    .await
    .map_err(|e| match e {
        store::WriteError::Taken => AppError::conflict(
            "deploy_taken",
            "A deploy with this name or this folder exists.",
        ),
        store::WriteError::UnknownApp => bad("app_id", "This is not one of your applications."),
        store::WriteError::Database(db) => internal(db),
    })?;
    if let Some(token) = &req.token {
        let sealed = state
            .master_key
            .seal(&store::token_context(id), token.as_bytes())
            .map_err(internal)?;
        store::set_token(&mut tx, id, Some(&sealed))
            .await
            .map_err(internal)?;
    }
    let row = store::get_in(&mut tx, tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("deploy"))?;
    if req.deploy_now.unwrap_or(true) {
        queue_deploy(&mut tx, Some(who.account_id()), &tenant, &row).await?;
    }
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("git_deploy.created", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("git_deploy", id)
            .metadata(serde_json::json!({ "name": req.name, "dest": req.dest })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let row = fresh(&state, tenant.id, id).await?;
    Ok((StatusCode::ACCEPTED, Json(view(row, Some(secret)))))
}

async fn fresh(state: &AppsState, tenant_id: Uuid, id: Uuid) -> Result<store::GitDeploy, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    store::get_in(&mut conn, tenant_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("deploy"))
}

/// Deploy again now.
#[utoipa::path(post, path = "/api/v1/client/git-deploys/{id}/deploy", tag = "apps",
    operation_id = "clientRunGitDeploy", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Deploy id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 202, description = "Queued", body = GitDeployView),
              (status = 404, description = "No such deploy", body = ProblemDetails, content_type = "application/problem+json"),
              (status = 409, description = "`deploy_running`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn deploy_now(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<GitDeployView>), AppError> {
    let (tenant, _plan) = feature(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can change its deploys.",
        ));
    }
    let mut tx = state.db.begin().await.map_err(internal)?;
    let row = store::get_in(&mut tx, tenant.id, id)
        .await
        .map_err(internal)?
        .filter(|d| d.status != "removing")
        .ok_or_else(|| AppError::not_found("deploy"))?;
    if row.status == "deploying" {
        return Err(AppError::conflict(
            "deploy_running",
            "This deploy is running; wait for it to end.",
        ));
    }
    queue_deploy(&mut tx, Some(who.account_id()), &tenant, &row).await?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("git_deploy.run", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("git_deploy", id),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let row = fresh(&state, tenant.id, id).await?;
    Ok((StatusCode::ACCEPTED, Json(view(row, None))))
}

/// Make a new webhook secret (the old one stops working); it is shown once, in the answer.
#[utoipa::path(post, path = "/api/v1/client/git-deploys/{id}/webhook-secret", tag = "apps",
    operation_id = "clientRotateGitWebhookSecret", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Deploy id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 200, description = "The deploy with its new secret", body = GitDeployView),
              (status = 404, description = "No such deploy", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn rotate_secret(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<Json<GitDeployView>, AppError> {
    let (tenant, _plan) = feature(&state, &who).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    store::get_in(&mut tx, tenant.id, id)
        .await
        .map_err(internal)?
        .filter(|d| d.status != "removing")
        .ok_or_else(|| AppError::not_found("deploy"))?;
    let secret = webhook_secret();
    let sealed = state
        .master_key
        .seal(&store::webhook_context(id), secret.as_bytes())
        .map_err(internal)?;
    builtpanel_database::sqlx::query("UPDATE git_deploys SET sealed_webhook = $2 WHERE id = $1")
        .bind(id)
        .bind(&sealed)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("git_deploy.secret_rotated", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("git_deploy", id),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    let row = fresh(&state, tenant.id, id).await?;
    Ok(Json(view(row, Some(secret))))
}

/// Remove a deploy (the releases and the link stay in your files).
#[utoipa::path(delete, path = "/api/v1/client/git-deploys/{id}", tag = "apps",
    operation_id = "clientRemoveGitDeploy", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Deploy id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses((status = 202, description = "Queued"),
              (status = 404, description = "No such deploy", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn remove(
    State(state): State<AppsState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let tenant = client_tenant(&state, &who).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let row = store::get_in(&mut tx, tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("deploy"))?;
    let op = Operation::RemoveGitDeploy {
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
            idempotency_key: &format!("gitdeploydel:{}:{}", id.simple(), Uuid::now_v7().simple()),
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
        &AuditEntry::new("git_deploy.removed", Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .target("git_deploy", id)
            .metadata(serde_json::json!({ "name": row.name })),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::ACCEPTED)
}

/// What the webhook answers.
#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookAnswer {
    /// Whether a deploy was started.
    pub deployed: bool,
    /// Why not, when not.
    pub reason: Option<String>,
}

/// Whether the request is signed with the deploy's secret, by either scheme: GitHub's
/// `X-Hub-Signature-256: sha256=<hex of the HMAC-SHA256 of the body>` or GitLab's
/// `X-Gitlab-Token: <the secret>`.
fn signed(headers: &HeaderMap, body: &[u8], secret: &str) -> bool {
    if let Some(theirs) = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("sha256="))
    {
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        return hex_decode(theirs).is_some_and(|sig| ring::hmac::verify(&key, body, &sig).is_ok());
    }
    if let Some(token) = headers.get("x-gitlab-token").and_then(|v| v.to_str().ok()) {
        // Compared as MACs under the secret, so that the comparison takes the same time
        // whatever the token is.
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let expected = ring::hmac::sign(&key, secret.as_bytes());
        return ring::hmac::verify(&key, token.as_bytes(), expected.as_ref()).is_ok();
    }
    false
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.len() > 128 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// A push to the repository starts the deploy. Signed with the deploy's secret; a push to
/// another branch, a ping, or a deploy that is already running starts nothing.
#[utoipa::path(post, path = "/api/v1/public/git-webhook/{id}", tag = "apps",
    operation_id = "gitWebhook",
    params(("id" = Uuid, Path, description = "Deploy id")),
    responses((status = 200, description = "Answered", body = WebhookAnswer),
              (status = 202, description = "A deploy was started", body = WebhookAnswer),
              (status = 401, description = "`bad_signature`", body = ProblemDetails, content_type = "application/problem+json")))]
pub async fn webhook(
    State(state): State<AppsState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<WebhookAnswer>), AppError> {
    let bad_signature =
        || AppError::authentication_failed("bad_signature", "The signature is not valid.");
    // The same answer whether the deploy exists or not.
    let sealed = store::sealed_webhook(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(bad_signature)?;
    let secret = String::from_utf8(
        state
            .master_key
            .open(&store::webhook_context(id), &sealed)
            .map_err(internal)?,
    )
    .map_err(internal)?;
    if !signed(&headers, &body, &secret) {
        return Err(bad_signature());
    }
    let answer = |deployed: bool, reason: &str| {
        Json(WebhookAnswer {
            deployed,
            reason: (!reason.is_empty()).then(|| reason.to_owned()),
        })
    };
    if headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|e| e != "push")
    {
        return Ok((StatusCode::OK, answer(false, "not a push")));
    }
    let pushed = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("ref").and_then(|r| r.as_str().map(str::to_owned)));
    let mut tx = state.db.begin().await.map_err(internal)?;
    let Some(row) = store::by_id_in(&mut tx, id).await.map_err(internal)? else {
        return Err(bad_signature());
    };
    if let Some(pushed) = pushed
        && pushed != format!("refs/heads/{}", row.branch)
    {
        return Ok((StatusCode::OK, answer(false, "another branch")));
    }
    if row.status == "deploying" || row.status == "removing" {
        return Ok((StatusCode::OK, answer(false, "a deploy is running")));
    }
    let Some(tenant) = tenants::get(&state.db, row.tenant_id)
        .await
        .map_err(internal)?
    else {
        return Err(bad_signature());
    };
    if tenant.status.as_str() != "active" {
        return Ok((StatusCode::OK, answer(false, "the account is not running")));
    }
    queue_deploy(&mut tx, None, &tenant, &row).await?;
    builtpanel_audit::insert(
        &mut *tx,
        &AuditEntry::new("git_deploy.webhook", Outcome::Success)
            .tenant(tenant.id)
            .target("git_deploy", id),
    )
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok((StatusCode::ACCEPTED, answer(true, "")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_webhook_is_believed_only_with_the_right_signature() {
        let secret = "s3cret";
        let body = br#"{"ref":"refs/heads/main"}"#;
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
        let hex: String = ring::hmac::sign(&key, body)
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let mut github = HeaderMap::new();
        github.insert(
            "x-hub-signature-256",
            format!("sha256={hex}").parse().unwrap(),
        );
        assert!(signed(&github, body, secret));
        assert!(!signed(&github, b"{}", secret), "another body");
        assert!(!signed(&github, body, "other"), "another secret");
        let mut gitlab = HeaderMap::new();
        gitlab.insert("x-gitlab-token", secret.parse().unwrap());
        assert!(signed(&gitlab, body, secret));
        gitlab.insert("x-gitlab-token", "nope".parse().unwrap());
        assert!(!signed(&gitlab, body, secret));
        assert!(
            !signed(&HeaderMap::new(), body, secret),
            "no signature at all"
        );
        let mut broken = HeaderMap::new();
        broken.insert("x-hub-signature-256", "sha256=zz".parse().unwrap());
        assert!(!signed(&broken, body, secret));
    }

    #[test]
    fn repositories_names_and_folders_are_checked() {
        assert!(valid_repo("https://github.com/alice/site.git"));
        for bad in [
            "http://x.example/a.git",
            "https://u:p@x.example/a",
            "git@github.com:a/b.git",
            "https://x y.example/a",
            "ssh://git@x.example/a",
        ] {
            assert!(!valid_repo(bad), "{bad}");
        }
        assert!(valid_name("my-site") && !valid_name("My") && !valid_name("a"));
        assert!(
            valid_dest("site")
                && valid_dest("apps/site")
                && !valid_dest(".bp-releases/x")
                && !valid_dest("../x")
        );
        assert!(valid_branch("main") && valid_branch("release/1.0") && !valid_branch("--x"));
    }
}
