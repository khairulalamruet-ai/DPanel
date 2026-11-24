//! SFTP and FTPS accounts (T08.12).
//!
//! | Method | Path |
//! |---|---|
//! | GET, POST | `/api/v1/client/ftp-accounts` |
//! | PUT, DELETE | `/api/v1/client/ftp-accounts/{id}` |
//! | PUT | `/api/v1/client/ftp-accounts/{id}/password` |
//!
//! Needs the plan's `ftp` feature. An account is a virtual user of the node's file
//! transfer server with a folder of the account's files as its home; its name starts
//! with the hosting account's name and an underscore. Everything is done on the node in
//! the background (`pending`, then `ready` or `failed`). A password is shown once, when
//! it is made or changed, and is kept sealed only until the node has it. Plain FTP (no TLS)
//! is off unless the panel's setting `ftp_plain_allowed` is on.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use builtpanel_database::{
    ftp_accounts::{self as store, AddError, FtpAccount},
    operations::{self, EnqueueError, NewOperation},
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::{DbPassword, Operation};
use builtpanel_rbac::Principal;
use builtpanel_settings::Scope;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{FilesState, Hosting, client_hosting_for, internal};

const OPERATION_TTL_SECS: i32 = 6 * 3600;

/// Routes for clients, with full paths.
pub fn client_router() -> Router<FilesState> {
    Router::new()
        .route(
            "/api/v1/client/ftp-accounts",
            get(list_accounts).post(create_account),
        )
        .route(
            "/api/v1/client/ftp-accounts/{id}",
            put(change_account).delete(delete_account),
        )
        .route(
            "/api/v1/client/ftp-accounts/{id}/password",
            put(set_password),
        )
}

/// An account.
#[derive(Debug, Serialize, ToSchema)]
pub struct FtpAccountView {
    /// Id.
    pub id: Uuid,
    /// The whole name (log in with it).
    #[schema(example = "alice_site")]
    pub name: String,
    /// Its home, relative to the account's files (`""`: all of them).
    pub home: String,
    /// May log in over SFTP.
    pub sftp: bool,
    /// May log in over FTP with TLS (FTPS).
    pub ftps: bool,
    /// May log in over FTP without TLS.
    pub ftp_plain: bool,
    /// May only read.
    pub read_only: bool,
    /// Disk limit in MB.
    pub quota_mb: Option<i64>,
    /// May log in at all.
    pub enabled: bool,
    /// `pending`, `ready`, `failed` or `deleting`.
    pub status: String,
}

fn view(a: FtpAccount) -> FtpAccountView {
    FtpAccountView {
        id: a.id,
        name: a.name,
        home: a.home,
        sftp: a.sftp,
        ftps: a.ftps,
        ftp_plain: a.ftp_plain,
        read_only: a.read_only,
        quota_mb: a.quota_mb,
        enabled: a.enabled,
        status: a.status,
    }
}

/// The accounts, and how to connect.
#[derive(Debug, Serialize, ToSchema)]
pub struct FtpAccountList {
    /// By name.
    pub items: Vec<FtpAccountView>,
    /// Most accounts the plan allows (`null`: no limit).
    pub limit: Option<i64>,
    /// What every name starts with (`alice_`).
    pub prefix: String,
    /// The address to connect to (empty when the panel has not been told).
    pub host: String,
    /// The SFTP port.
    pub sftp_port: i64,
    /// The FTP port (use TLS: explicit FTPS).
    pub ftp_port: i64,
    /// Whether plain FTP (no TLS) may be allowed to accounts.
    pub plain_allowed: bool,
}

/// A new account, with the password if one was made.
#[derive(Debug, Serialize, ToSchema)]
pub struct FtpAccountCreated {
    /// The account.
    pub account: FtpAccountView,
    /// The password, shown this once (only when it was made here).
    pub password: Option<String>,
}

async fn setting(state: &FilesState, key: &str) -> Result<Option<serde_json::Value>, AppError> {
    let mut conn = state.db.acquire().await.map_err(internal)?;
    let values = builtpanel_settings::get(&state.settings, &mut conn, "files", Scope::Global)
        .await
        .map_err(internal)?;
    Ok(values.get(key).cloned())
}

fn limit(h: &Hosting) -> Option<i64> {
    let v = h
        .plan
        .limits
        .get("max_ftp_accounts")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(5);
    (v >= 0).then_some(v)
}

/// The accounts of the signed-in client.
#[utoipa::path(
    get,
    path = "/api/v1/client/ftp-accounts",
    tag = "files",
    operation_id = "clientListFtpAccounts",
    security(("session" = [])),
    responses(
        (status = 200, description = "The accounts", body = FtpAccountList),
        (status = 403, description = "`client_only` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_accounts(
    State(state): State<FilesState>,
    who: Principal,
) -> Result<Json<FtpAccountList>, AppError> {
    let h = client_hosting_for(&state, &who, "ftp").await?;
    let items = store::list(&state.db, h.tenant.id)
        .await
        .map_err(internal)?;
    let int = |v: Option<serde_json::Value>, d: i64| v.and_then(|v| v.as_i64()).unwrap_or(d);
    Ok(Json(FtpAccountList {
        items: items.into_iter().map(view).collect(),
        limit: limit(&h),
        prefix: format!("{}_", h.tenant.username),
        host: setting(&state, "connect_host")
            .await?
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        sftp_port: int(setting(&state, "sftp_port").await?, 2222),
        ftp_port: int(setting(&state, "ftp_port").await?, 2121),
        plain_allowed: setting(&state, "ftp_plain_allowed")
            .await?
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }))
}

/// An account to make.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddFtpAccount {
    /// The name; the hosting account's prefix is put in front when it is not there.
    #[schema(example = "site")]
    pub name: String,
    /// The home, relative to the account's files (default: all of them).
    #[serde(default)]
    pub home: String,
    /// The password; a strong one is made (and shown once) when it is left out.
    pub password: Option<String>,
    /// Allow SFTP (default yes).
    pub sftp: Option<bool>,
    /// Allow FTP with TLS (default yes).
    pub ftps: Option<bool>,
    /// Allow FTP without TLS (default no; refused unless the panel allows it).
    pub ftp_plain: Option<bool>,
    /// Only read (default no).
    #[serde(default)]
    pub read_only: bool,
    /// Disk limit in MB.
    pub quota_mb: Option<i64>,
}

fn full_name(prefix: &str, given: &str) -> Result<String, AppError> {
    let given = given.trim().to_ascii_lowercase();
    let name = if given.starts_with(prefix) {
        given
    } else {
        format!("{prefix}{given}")
    };
    let ok = name.len() <= 64
        && name.len() > prefix.len()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(name)
    } else {
        Err(AppError::validation(vec![FieldError::new(
            "name",
            "invalid_name",
            "Names have lower-case letters, digits, `_`, `-` and `.` and are at most 64 characters with the prefix.",
        )]))
    }
}

/// A folder of the account's files, as text.
fn check_home(home: &str) -> Result<String, AppError> {
    let bad = || {
        AppError::validation(vec![FieldError::new(
            "home",
            "invalid_path",
            "The home must be a folder of your files, without `..`.",
        )])
    };
    if home.len() > 1024 || home.contains('\0') || home.starts_with('/') {
        return Err(bad());
    }
    let parts: Vec<&str> = home
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.len() > 32
        || parts
            .iter()
            .any(|p| *p == ".." || p.len() > 255 || p.chars().any(char::is_control))
        || parts.first() == Some(&".trash")
    {
        return Err(bad());
    }
    Ok(parts.join("/"))
}

fn password_for(given: Option<String>) -> Result<(String, bool), AppError> {
    match given {
        Some(p) => {
            builtpanel_domain::databases::check_password(&p).map_err(|e| {
                AppError::validation(vec![FieldError::new("password", e.code(), e.to_string())])
            })?;
            Ok((p, false))
        }
        None => loop {
            let a = Uuid::new_v4().simple().to_string();
            let b = Uuid::new_v4().simple().to_string();
            let candidate = format!("{}-{}", a[..10].to_uppercase(), &b[..14]);
            if builtpanel_domain::databases::check_password(&candidate).is_ok() {
                return Ok((candidate, true));
            }
        },
    }
}

async fn check_plain(state: &FilesState, ftp_plain: bool) -> Result<(), AppError> {
    if ftp_plain
        && !setting(state, "ftp_plain_allowed")
            .await?
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        return Err(AppError::validation(vec![FieldError::new(
            "ftp_plain",
            "plain_ftp_disabled",
            "FTP without TLS is not allowed on this panel.",
        )]));
    }
    Ok(())
}

/// Queues the node's work (and keeps the sealed password), inside the caller's transaction.
async fn queue(
    state: &FilesState,
    tx: &mut builtpanel_database::sqlx::PgConnection,
    h: &Hosting,
    who: &Principal,
    a: &FtpAccount,
    password: Option<&str>,
) -> Result<(), AppError> {
    if let Some(p) = password {
        let sealed = state
            .master_key
            .seal(&store::password_context(a.id), p.as_bytes())
            .map_err(internal)?;
        store::keep_password(tx, a.id, &sealed)
            .await
            .map_err(internal)?;
    }
    let op = Operation::UpsertFtpAccount {
        tenant_id: h.tenant.id,
        uid: u32::try_from(h.tenant.uid).map_err(internal)?,
        name: a.name.clone(),
        home: a.home.clone(),
        // Not in the ledger: the dispatcher opens the sealed one at send time.
        password: None::<DbPassword>,
        sftp: a.sftp,
        ftps: a.ftps,
        ftp_plain: a.ftp_plain,
        read_only: a.read_only,
        quota_mb: a.quota_mb.and_then(|q| u64::try_from(q).ok()),
        enabled: a.enabled,
    };
    enqueue(
        tx,
        h,
        who,
        &op,
        format!("ftp:{}:{}", a.id.simple(), Uuid::now_v7().simple()),
    )
    .await
}

async fn enqueue(
    tx: &mut builtpanel_database::sqlx::PgConnection,
    h: &Hosting,
    who: &Principal,
    op: &Operation,
    key: String,
) -> Result<(), AppError> {
    let payload = serde_json::to_value(op).map_err(internal)?;
    operations::enqueue_in(
        tx,
        &NewOperation {
            node_id: h.tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(h.tenant.id),
            payload: &payload,
            idempotency_key: &key,
            created_by: Some(who.account_id()),
            ttl_secs: OPERATION_TTL_SECS,
            max_attempts: 5,
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
    Ok(())
}

async fn audit(
    tx: &mut builtpanel_database::sqlx::PgConnection,
    action: &'static str,
    who: &Principal,
    h: &Hosting,
    id: Uuid,
    name: &str,
) -> Result<(), AppError> {
    builtpanel_audit::insert(
        &mut *tx,
        &builtpanel_audit::AuditEntry::new(action, builtpanel_audit::Outcome::Success)
            .actor(who.account_id())
            .tenant(h.tenant.id)
            .target("ftp_account", id)
            .metadata(serde_json::json!({ "name": name })),
    )
    .await
    .map_err(internal)
}

/// Make an account.
#[utoipa::path(
    post,
    path = "/api/v1/client/ftp-accounts",
    tag = "files",
    operation_id = "clientCreateFtpAccount",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = AddFtpAccount,
    responses(
        (status = 201, description = "Queued; `status` becomes `ready` when the node has made it", body = FtpAccountCreated),
        (status = 403, description = "`client_only`, `csrf_failed` or `feature_disabled`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`limit_reached`, `name_taken` or `tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_name`, `invalid_path`, `plain_ftp_disabled`, a bad password or quota", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create_account(
    State(state): State<FilesState>,
    who: Principal,
    Json(req): Json<AddFtpAccount>,
) -> Result<(StatusCode, Json<FtpAccountCreated>), AppError> {
    let h = client_hosting_for(&state, &who, "ftp").await?;
    let name = full_name(&format!("{}_", h.tenant.username), &req.name)?;
    let home = check_home(&req.home)?;
    let protocols = (
        req.sftp.unwrap_or(true),
        req.ftps.unwrap_or(true),
        req.ftp_plain.unwrap_or(false),
    );
    if !(protocols.0 || protocols.1 || protocols.2) {
        return Err(AppError::validation(vec![FieldError::new(
            "sftp",
            "no_protocol",
            "Allow at least one of SFTP, FTPS and FTP.",
        )]));
    }
    check_plain(&state, protocols.2).await?;
    if req.quota_mb.is_some_and(|q| q <= 0) {
        return Err(AppError::validation(vec![FieldError::new(
            "quota_mb",
            "invalid_quota",
            "The limit is at least 1 MB.",
        )]));
    }
    let (password, generated) = password_for(req.password)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    builtpanel_database::domains::lock_tenant(&mut tx, h.tenant.id)
        .await
        .map_err(internal)?;
    if let Some(max) = limit(&h)
        && store::count(&mut tx, h.tenant.id).await.map_err(internal)? >= max
    {
        return Err(AppError::conflict(
            "limit_reached",
            format!("This plan allows {max} accounts."),
        ));
    }
    let made = store::create(
        &mut tx,
        h.tenant.id,
        &name,
        &home,
        protocols,
        req.read_only,
        req.quota_mb,
    )
    .await
    .map_err(|e| match e {
        AddError::Taken => AppError::conflict("name_taken", "This name is taken."),
        AddError::Database(db) => internal(db),
    })?;
    queue(&state, &mut tx, &h, &who, &made, Some(&password)).await?;
    audit(
        &mut tx,
        "ftp_account.created",
        &who,
        &h,
        made.id,
        &made.name,
    )
    .await?;
    tx.commit().await.map_err(internal)?;
    Ok((
        StatusCode::CREATED,
        Json(FtpAccountCreated {
            account: view(made),
            password: generated.then_some(password),
        }),
    ))
}

/// What to change; what is left out stays.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeFtpAccount {
    /// The home.
    pub home: Option<String>,
    /// Allow SFTP.
    pub sftp: Option<bool>,
    /// Allow FTP with TLS.
    pub ftps: Option<bool>,
    /// Allow FTP without TLS.
    pub ftp_plain: Option<bool>,
    /// Only read.
    pub read_only: Option<bool>,
    /// Disk limit in MB (`0` removes it).
    pub quota_mb: Option<i64>,
    /// May log in at all.
    pub enabled: Option<bool>,
}

/// Change an account.
#[utoipa::path(
    put,
    path = "/api/v1/client/ftp-accounts/{id}",
    tag = "files",
    operation_id = "clientChangeFtpAccount",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Account id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = ChangeFtpAccount,
    responses(
        (status = 200, description = "Queued", body = FtpAccountView),
        (status = 404, description = "No such account", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`invalid_path`, `plain_ftp_disabled`, `no_protocol` or a bad quota", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn change_account(
    State(state): State<FilesState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<ChangeFtpAccount>,
) -> Result<Json<FtpAccountView>, AppError> {
    let h = client_hosting_for(&state, &who, "ftp").await?;
    let a = store::get(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .filter(|a| a.status != "deleting")
        .ok_or_else(|| AppError::not_found("ftp_account"))?;
    let home = match &req.home {
        Some(home) => check_home(home)?,
        None => a.home.clone(),
    };
    let protocols = (
        req.sftp.unwrap_or(a.sftp),
        req.ftps.unwrap_or(a.ftps),
        req.ftp_plain.unwrap_or(a.ftp_plain),
    );
    if !(protocols.0 || protocols.1 || protocols.2) {
        return Err(AppError::validation(vec![FieldError::new(
            "sftp",
            "no_protocol",
            "Allow at least one of SFTP, FTPS and FTP.",
        )]));
    }
    if protocols.2 && !a.ftp_plain {
        check_plain(&state, true).await?;
    }
    let quota = match req.quota_mb {
        Some(0) => None,
        Some(q) if q < 0 => {
            return Err(AppError::validation(vec![FieldError::new(
                "quota_mb",
                "invalid_quota",
                "The limit is at least 1 MB.",
            )]));
        }
        Some(q) => Some(q),
        None => a.quota_mb,
    };
    let mut tx = state.db.begin().await.map_err(internal)?;
    store::change(
        &mut tx,
        id,
        &home,
        protocols,
        req.read_only.unwrap_or(a.read_only),
        quota,
        req.enabled.unwrap_or(a.enabled),
    )
    .await
    .map_err(internal)?;
    let changed = store::get(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("ftp_account"))?;
    // The change is read after it is written (the transaction is not committed yet,
    // so the old values are in `changed`; the node is told the new ones).
    let now = FtpAccount {
        home,
        sftp: protocols.0,
        ftps: protocols.1,
        ftp_plain: protocols.2,
        read_only: req.read_only.unwrap_or(a.read_only),
        quota_mb: quota,
        enabled: req.enabled.unwrap_or(a.enabled),
        status: "pending".to_owned(),
        ..changed
    };
    queue(&state, &mut tx, &h, &who, &now, None).await?;
    audit(&mut tx, "ftp_account.changed", &who, &h, id, &now.name).await?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(view(now)))
}

/// A new password.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewFtpPassword {
    /// The password; a strong one is made (and shown once) when it is left out.
    pub password: Option<String>,
}

/// Change an account's password.
#[utoipa::path(
    put,
    path = "/api/v1/client/ftp-accounts/{id}/password",
    tag = "files",
    operation_id = "clientSetFtpPassword",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Account id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = NewFtpPassword,
    responses(
        (status = 200, description = "Queued; the new password if it was made here", body = FtpAccountCreated),
        (status = 404, description = "No such account", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn set_password(
    State(state): State<FilesState>,
    who: Principal,
    Path(id): Path<Uuid>,
    Json(req): Json<NewFtpPassword>,
) -> Result<Json<FtpAccountCreated>, AppError> {
    let h = client_hosting_for(&state, &who, "ftp").await?;
    let a = store::get(&state.db, h.tenant.id, id)
        .await
        .map_err(internal)?
        .filter(|a| a.status != "deleting")
        .ok_or_else(|| AppError::not_found("ftp_account"))?;
    let (password, generated) = password_for(req.password)?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    queue(&state, &mut tx, &h, &who, &a, Some(&password)).await?;
    audit(
        &mut tx,
        "ftp_account.password_changed",
        &who,
        &h,
        id,
        &a.name,
    )
    .await?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(FtpAccountCreated {
        account: view(a),
        password: generated.then_some(password),
    }))
}

/// Remove an account.
#[utoipa::path(
    delete,
    path = "/api/v1/client/ftp-accounts/{id}",
    tag = "files",
    operation_id = "clientDeleteFtpAccount",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Account id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 202, description = "Queued; the account disappears from the list when the node has removed it"),
        (status = 404, description = "No such account", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn delete_account(
    State(state): State<FilesState>,
    who: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let h = client_hosting_for(&state, &who, "ftp").await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    let gone = store::mark_deleting(&mut tx, h.tenant.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("ftp_account"))?;
    enqueue(
        &mut tx,
        &h,
        &who,
        &Operation::DropFtpAccount {
            name: gone.name.clone(),
        },
        format!("ftpdrop:{}", gone.id.simple()),
    )
    .await?;
    audit(&mut tx, "ftp_account.deleted", &who, &h, id, &gone.name).await?;
    tx.commit().await.map_err(internal)?;
    Ok(StatusCode::ACCEPTED)
}
