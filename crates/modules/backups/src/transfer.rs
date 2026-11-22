//! Moving a hosting account from one node to another (T15.07).
//!
//! The nodes never talk to each other: everything goes through a backup destination both of
//! them reach, with the backup code doing the copying. The account is copied **while it still
//! runs**, so that what the last pass has to carry is small:
//!
//! 1. the account is set up on the new node (empty);
//! 2. a full backup is made on the old node (files, databases, and the database users'
//!    password hashes, in the encrypted setup data) and restored on the new one: the first copy;
//! 3. on `cutover` the account is **frozen** on the old node (its containers stop; this is when
//!    the site goes off), a second backup carries only the files made or changed since the first
//!    copy began plus every database again, and it is restored on the new node;
//! 4. the account is switched: it belongs to the new node, its web routes, PHP containers, cron
//!    jobs, database grants and remote addresses are sent there, and its DNS records that name
//!    the old node's address name the new node's;
//! 5. the old node's copy is kept until somebody asks for it to be removed (`cleanup`).
//!
//! A step that fails before the switch puts the old node back to work (the account is resumed
//! there) and the transfer is `failed`; nothing on the old node was changed by it. Work is a loop
//! of [`tick`]s that keeps no state of its own: the transfer's row says where it is, and every
//! operation it queues has a key made of the transfer's id, so a crash between two steps is
//! repaired by the next tick.
//!
//! Not carried yet, and refused when the account has them: applications and containers, cache
//! add-ons, FTP accounts, PostgreSQL databases and databases on a server of the account's own.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_auth_core::secretbox::MasterKey;
use builtpanel_backup::crypto::new_key;
use builtpanel_database::{
    backups as backup_store,
    operations::{self, NewOperation, Status},
    sqlx::PgPool,
    tenant_db, tenants,
    transfers::{self as store, CreateError, Extras, Transfer},
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_node_protocol::Operation;
use builtpanel_rbac::{
    Principal, Require,
    permissions::{BackupsManage, BackupsRead},
};
use builtpanel_session::rfc3339;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::BackupsState;

/// A file changed a moment before the first copy began may be missed by it; the last pass looks
/// this far back.
const SLACK_SECS: i64 = 30;
const OP_TTL_SECS: i32 = 6 * 3600;

/// What went wrong in a tick (the loop logs it and goes on).
#[derive(Debug, thiserror::Error)]
pub enum TickError {
    /// The database.
    #[error(transparent)]
    Database(#[from] builtpanel_database::sqlx::Error),
    /// The ledger refused an operation.
    #[error("cannot queue an operation: {0}")]
    Queue(String),
    /// A key could not be made or sealed.
    #[error("cannot make or seal a key")]
    Key,
}

async fn queue(
    db: &PgPool,
    node_id: Uuid,
    tenant_id: Uuid,
    op: &Operation,
    key: &str,
) -> Result<Uuid, TickError> {
    let payload = serde_json::to_value(op).map_err(|e| TickError::Queue(e.to_string()))?;
    operations::enqueue(
        db,
        &NewOperation {
            node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant_id),
            payload: &payload,
            idempotency_key: key,
            created_by: None,
            ttl_secs: OP_TTL_SECS,
            max_attempts: 5,
        },
    )
    .await
    .map(|queued| queued.operation().id)
    .map_err(|e| TickError::Queue(e.to_string()))
}

/// How a thing the transfer waits for stands.
enum Wait {
    Going,
    Done,
    Failed(String),
}

async fn operation_wait(db: &PgPool, id: Uuid) -> Result<Wait, TickError> {
    Ok(match operations::get(db, id).await? {
        Some(op) => match op.status {
            Status::Succeeded => Wait::Done,
            Status::Failed => Wait::Failed("an operation on a node failed".to_owned()),
            _ => Wait::Going,
        },
        None => Wait::Failed("an operation is gone".to_owned()),
    })
}

async fn backup_wait(db: &PgPool, id: Uuid) -> Result<Wait, TickError> {
    let row = builtpanel_database::sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, error FROM backups WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(match row {
        Some((status, _)) if status == "ready" => Wait::Done,
        Some((status, error)) if status == "failed" => {
            Wait::Failed(error.unwrap_or_else(|| "the backup failed".to_owned()))
        }
        Some(_) => Wait::Going,
        None => Wait::Failed("the backup is gone".to_owned()),
    })
}

async fn restore_wait(db: &PgPool, id: Uuid) -> Result<Wait, TickError> {
    let row = builtpanel_database::sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, error FROM backup_restores WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(match row {
        Some((status, _)) if status == "done" => Wait::Done,
        Some((status, error)) if status == "failed" => {
            Wait::Failed(error.unwrap_or_else(|| "the restore failed".to_owned()))
        }
        Some(_) => Wait::Going,
        None => Wait::Failed("the restore is gone".to_owned()),
    })
}

/// Makes a backup of the account on the old node, with its key.
async fn start_backup(
    db: &PgPool,
    master_key: &MasterKey,
    t: &Transfer,
    files_since: Option<i64>,
) -> Result<Uuid, TickError> {
    let tenant = tenants::get(db, t.tenant_id)
        .await?
        .ok_or_else(|| TickError::Queue("the account is gone".to_owned()))?;
    let id = backup_store::new_backup(
        db,
        tenant.id,
        &tenant.username,
        "manual",
        t.destination_id,
        t.created_by,
    )
    .await?;
    let key = new_key().map_err(|_| TickError::Key)?;
    let sealed = master_key
        .seal(&backup_store::key_context(id), &key)
        .map_err(|_| TickError::Key)?;
    backup_store::keep_key(db, id, &sealed).await?;
    store::add_extras(
        db,
        id,
        t.id,
        &Extras {
            node_id: t.from_node,
            files_since,
            with_db_users: true,
            create_databases: false,
            restore_db_users: false,
        },
    )
    .await?;
    Ok(id)
}

/// Puts a backup of the transfer back on the new node.
async fn start_restore(db: &PgPool, t: &Transfer, backup_id: Uuid) -> Result<Uuid, TickError> {
    let databases = tenant_db::move_set(db, t.tenant_id, "mysql")
        .await?
        .databases;
    let id = backup_store::new_restore(
        db,
        &backup_store::NewRestore {
            backup_id,
            tenant_id: t.tenant_id,
            files: true,
            folder: None,
            databases: &databases,
            dns: false,
            snapshot: false,
            verify_only: false,
            created_by: t.created_by,
        },
    )
    .await
    .map_err(|e| TickError::Queue(e.to_string()))?;
    store::add_extras(
        db,
        id,
        t.id,
        &Extras {
            node_id: t.to_node,
            files_since: None,
            with_db_users: false,
            create_databases: true,
            restore_db_users: true,
        },
    )
    .await?;
    Ok(id)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// Gives the old node its account back when a step after the freeze failed, and ends the
/// transfer as failed.
async fn fail(db: &PgPool, t: &Transfer, why: &str) -> Result<(), TickError> {
    if t.frozen_at.is_some()
        && t.state != "switched"
        && let Some(tenant) = tenants::get(db, t.tenant_id).await?
    {
        queue(
            db,
            t.from_node,
            t.tenant_id,
            &Operation::ResumeTenant {
                username: tenant.username,
            },
            &format!("transfer:{}:resume-old", t.id.simple()),
        )
        .await?;
    }
    let why: String = why.chars().take(400).collect();
    store::set_state(db, t.id, "failed", None, Some(&why)).await?;
    Ok(())
}

/// Moves every transfer one step on where what it waits for is over.
///
/// # Errors
/// [`TickError`].
pub async fn tick(db: &PgPool, master_key: &MasterKey) -> Result<(), TickError> {
    // One transfer's trouble does not hold the others back; the first error is reported.
    let mut first = None;
    for t in store::active(db).await? {
        if let Err(error) = step(db, master_key, &t).await {
            first.get_or_insert(error);
        }
    }
    first.map_or(Ok(()), Err)
}

#[allow(clippy::too_many_lines)]
async fn step(db: &PgPool, master_key: &MasterKey, t: &Transfer) -> Result<(), TickError> {
    let Some(tenant) = tenants::get(db, t.tenant_id).await? else {
        return fail(db, t, "the account is gone").await;
    };
    match t.state.as_str() {
        "requested" => {
            let plan = builtpanel_database::plans::get(db, tenant.plan_id)
                .await?
                .ok_or_else(|| TickError::Queue("the plan is gone".to_owned()))?;
            let id = queue(
                db,
                t.to_node,
                t.tenant_id,
                &Operation::SetUpTenant {
                    tenant_id: tenant.id,
                    username: tenant.username.clone(),
                    uid: u32::try_from(tenant.uid).unwrap_or(0),
                    disk_bytes: plan.disk_bytes.and_then(|v| u64::try_from(v).ok()),
                    inodes: plan.inodes.and_then(|v| u64::try_from(v).ok()),
                    cpu_millis: plan.cpu_millis.and_then(|v| u32::try_from(v).ok()),
                    memory_bytes: plan.memory_bytes.and_then(|v| u64::try_from(v).ok()),
                    pids: plan.pids.and_then(|v| u32::try_from(v).ok()),
                    skeleton: Vec::new(),
                },
                &format!("transfer:{}:setup", t.id.simple()),
            )
            .await?;
            store::set_state(db, t.id, "provisioning", Some(id), None).await?;
        }
        "provisioning" => match wait_for(db, t, Thing::Operation).await? {
            Wait::Going => {}
            Wait::Failed(why) => fail(db, t, &why).await?,
            Wait::Done => {
                let started = now_secs();
                let id = start_backup(db, master_key, t, None).await?;
                store::keep(db, t.id, Some(id), None, Some(started), None, None).await?;
                store::set_state(db, t.id, "precopy_backup", Some(id), None).await?;
            }
        },
        "precopy_backup" => match wait_for(db, t, Thing::Backup).await? {
            Wait::Going => {}
            Wait::Failed(why) => fail(db, t, &why).await?,
            Wait::Done => {
                let backup = t.first_backup.unwrap_or_default();
                let id = start_restore(db, t, backup).await?;
                store::set_state(db, t.id, "precopy_restore", Some(id), None).await?;
            }
        },
        "precopy_restore" => match wait_for(db, t, Thing::Restore).await? {
            Wait::Going => {}
            Wait::Failed(why) => fail(db, t, &why).await?,
            Wait::Done => store::set_state(db, t.id, "precopied", None, None).await?,
        },
        // `precopied` is only looked at here when the transfer goes on by itself or has been
        // told to (`cutover` sets the flag).
        "precopied" => {
            let id = queue(
                db,
                t.from_node,
                t.tenant_id,
                &Operation::SuspendTenant {
                    username: tenant.username.clone(),
                },
                &format!("transfer:{}:freeze", t.id.simple()),
            )
            .await?;
            store::keep(db, t.id, None, None, None, Some(now_secs()), None).await?;
            store::set_state(db, t.id, "freezing", Some(id), None).await?;
        }
        "freezing" => match wait_for(db, t, Thing::Operation).await? {
            Wait::Going => {}
            Wait::Failed(why) => fail(db, t, &why).await?,
            Wait::Done => {
                let since = t.copy_started_at.map(|s| s - SLACK_SECS);
                let id = start_backup(db, master_key, t, since).await?;
                store::keep(db, t.id, None, Some(id), None, None, None).await?;
                store::set_state(db, t.id, "delta_backup", Some(id), None).await?;
            }
        },
        "delta_backup" => match wait_for(db, t, Thing::Backup).await? {
            Wait::Going => {}
            Wait::Failed(why) => fail(db, t, &why).await?,
            Wait::Done => {
                let backup = t.delta_backup.unwrap_or_default();
                let id = start_restore(db, t, backup).await?;
                store::set_state(db, t.id, "delta_restore", Some(id), None).await?;
            }
        },
        "delta_restore" => match wait_for(db, t, Thing::Restore).await? {
            Wait::Going => {}
            Wait::Failed(why) => fail(db, t, &why).await?,
            Wait::Done => switch(db, t, &tenant).await?,
        },
        "cleaning" => match wait_for(db, t, Thing::Operation).await? {
            Wait::Going => {}
            Wait::Failed(why) => fail(db, t, &why).await?,
            Wait::Done => {
                // The copies made for the transfer are no longer wanted.
                for backup in [t.first_backup, t.delta_backup].into_iter().flatten() {
                    backup_store::mark_deleting(db, backup).await?;
                }
                store::set_state(db, t.id, "done", None, None).await?;
            }
        },
        "cancelling" => match wait_for(db, t, Thing::Operation).await? {
            Wait::Going => {}
            Wait::Failed(why) => fail(db, t, &why).await?,
            Wait::Done => {
                for backup in [t.first_backup, t.delta_backup].into_iter().flatten() {
                    backup_store::mark_deleting(db, backup).await?;
                }
                store::set_state(db, t.id, "cancelled", None, None).await?;
            }
        },
        _ => {}
    }
    Ok(())
}

/// What a transfer waits for.
#[derive(Clone, Copy)]
enum Thing {
    Operation,
    Backup,
    Restore,
}

async fn wait_for(db: &PgPool, t: &Transfer, thing: Thing) -> Result<Wait, TickError> {
    match (t.waiting_for, thing) {
        (Some(id), Thing::Operation) => operation_wait(db, id).await,
        (Some(id), Thing::Backup) => backup_wait(db, id).await,
        (Some(id), Thing::Restore) => restore_wait(db, id).await,
        (None, _) => Ok(Wait::Failed("the transfer waits for nothing".to_owned())),
    }
}

/// The account now belongs to the new node, and everything the new node needs to serve it is
/// sent there, in one transaction.
async fn switch(db: &PgPool, t: &Transfer, tenant: &tenants::Tenant) -> Result<(), TickError> {
    let mut tx = db.begin().await?;
    // 1. Whose it is, and what is sent again: web routes (the dispatcher sends a tenant whose
    // routes version moved), PHP containers (a state version nobody applied yet).
    builtpanel_database::sqlx::query(
        "UPDATE tenants SET node_id = $2, routes_version = routes_version + 1,
                php_state_version = php_state_version + 1
         WHERE id = $1",
    )
    .bind(t.tenant_id)
    .bind(t.to_node)
    .execute(&mut *tx)
    .await?;
    // 2. DNS: records that name the old node's address name the new node's.
    let old: Option<String> = builtpanel_database::sqlx::query_scalar(
        "SELECT host(public_ip) FROM nodes WHERE id = $1 AND public_ip IS NOT NULL",
    )
    .bind(t.from_node)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    let new: Option<String> = builtpanel_database::sqlx::query_scalar(
        "SELECT host(public_ip) FROM nodes WHERE id = $1 AND public_ip IS NOT NULL",
    )
    .bind(t.to_node)
    .fetch_optional(&mut *tx)
    .await?
    .flatten();
    if let (Some(old), Some(new)) = (old, new) {
        let zones: Vec<Uuid> = builtpanel_database::sqlx::query_scalar(
            "UPDATE dns_records r SET content = $3, updated_at = now()
             FROM dns_zones z JOIN domains d ON d.id = z.domain_id
             WHERE r.zone_id = z.id AND d.tenant_id = $1 AND r.rtype IN ('A', 'AAAA')
               AND r.content = $2
             RETURNING r.zone_id",
        )
        .bind(t.tenant_id)
        .bind(&old)
        .bind(&new)
        .fetch_all(&mut *tx)
        .await?;
        let mut seen = Vec::new();
        for zone in zones {
            if !seen.contains(&zone) {
                builtpanel_database::dns::mark_changed(&mut tx, zone).await?;
                seen.push(zone);
            }
        }
    }
    tx.commit().await?;

    // 3. What the new node does not know yet: the jobs, the grants, the remote addresses.
    let key = |what: &str| format!("transfer:{}:{what}", t.id.simple());
    let jobs = builtpanel_database::cron_jobs::list(db, t.tenant_id).await?;
    queue(
        db,
        t.to_node,
        t.tenant_id,
        &Operation::SyncCronJobs {
            tenant_id: t.tenant_id,
            username: tenant.username.clone(),
            jobs: jobs
                .iter()
                .map(|j| builtpanel_node_protocol::CronJobSpec {
                    id: j.id,
                    schedule: j.schedule.clone(),
                    command: j.command.clone(),
                    enabled: j.enabled,
                    timeout_seconds: u32::try_from(j.timeout_seconds).unwrap_or(300),
                })
                .collect(),
        },
        &key("cron"),
    )
    .await?;
    let set = tenant_db::move_set(db, t.tenant_id, "mysql").await?;
    for (database, user, access) in &set.grants {
        queue(
            db,
            t.to_node,
            t.tenant_id,
            &Operation::GrantDb {
                database: database.clone(),
                user: user.clone(),
                access: access.clone(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
            &key(&format!("grant:{database}:{user}")),
        )
        .await?;
    }
    for (user, cidrs) in &set.remote {
        queue(
            db,
            t.to_node,
            t.tenant_id,
            &Operation::SetDbRemoteHosts {
                username: tenant.username.clone(),
                name: user.clone(),
                cidrs: cidrs.clone(),
                engine: builtpanel_node_protocol::DbEngine::Mysql,
                dedicated: None,
            },
            &key(&format!("remote:{user}")),
        )
        .await?;
    }
    // Both nodes' lists of who may reach their database server from outside change.
    for node in [t.from_node, t.to_node] {
        let mut conn = db.acquire().await?;
        let cidrs = builtpanel_database::databases::remote_union(&mut conn, node, "mysql").await?;
        drop(conn);
        queue(
            db,
            node,
            t.tenant_id,
            &Operation::SetDbRemoteAccess {
                cidrs,
                engine: builtpanel_node_protocol::DbEngine::Mysql,
            },
            &key(&format!("access:{}", node.simple())),
        )
        .await?;
    }
    let downtime = t
        .frozen_at
        .map(|at| i32::try_from((now_secs() - at).max(0)).unwrap_or(i32::MAX));
    store::keep(db, t.id, None, None, None, None, downtime).await?;
    store::set_state(db, t.id, "switched", None, None).await?;
    Ok(())
}

// ------------------------------------------------------------------------------ API

/// A transfer of an account to another node.
#[derive(Debug, Serialize, ToSchema)]
pub struct NodeTransferView {
    /// Id.
    pub id: Uuid,
    /// The account.
    pub tenant_id: Uuid,
    /// The node it is leaving.
    pub from_node: Uuid,
    /// The node it goes to.
    pub to_node: Uuid,
    /// The backup destination the copies pass through.
    pub destination_id: Uuid,
    /// Whether it goes on to the last pass by itself.
    pub auto_cutover: bool,
    /// `requested`, `provisioning`, `precopy_backup`, `precopy_restore`, `precopied` (waiting
    /// for `cutover`), `freezing`, `delta_backup`, `delta_restore`, `switching`, `switched`
    /// (the old copy is still there), `cleaning`, `done`, `cancelling`, `cancelled` or
    /// `failed`.
    pub state: String,
    /// Seconds the site was off (when it is over).
    pub downtime_secs: Option<i32>,
    /// What went wrong.
    pub error: Option<String>,
    /// When it was asked for (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub created_at: String,
    /// When it last changed (RFC 3339, UTC).
    #[schema(format = DateTime)]
    pub updated_at: String,
}

fn view(t: Transfer) -> NodeTransferView {
    NodeTransferView {
        id: t.id,
        tenant_id: t.tenant_id,
        from_node: t.from_node,
        to_node: t.to_node,
        destination_id: t.destination_id,
        auto_cutover: t.auto_cutover,
        state: t.state,
        downtime_secs: t.downtime_secs,
        error: t.error,
        created_at: rfc3339(t.created_at),
        updated_at: rfc3339(t.updated_at),
    }
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

fn entry(action: &'static str, who: &Principal, id: Uuid) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target("node_transfer", id)
}

/// What to move, and where.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartTransfer {
    /// The hosting account.
    pub tenant_id: Uuid,
    /// The node it goes to.
    pub to_node_id: Uuid,
    /// The backup destination to pass the copies through (the default one when left out).
    pub destination_id: Option<Uuid>,
    /// Go on to the last pass (and the switch) as soon as the first copy is there.
    #[serde(default)]
    pub auto_cutover: bool,
}

/// The newest transfers.
#[utoipa::path(
    get,
    path = "/api/v1/admin/node-transfers",
    tag = "backups",
    operation_id = "adminListNodeTransfers",
    security(("session" = [])),
    responses(
        (status = 200, description = "The newest first", body = Vec<NodeTransferView>),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<BackupsState>,
    _who: Require<BackupsRead>,
) -> Result<Json<Vec<NodeTransferView>>, AppError> {
    Ok(Json(
        store::list(&state.db, 100)
            .await
            .map_err(internal)?
            .into_iter()
            .map(view)
            .collect(),
    ))
}

/// One transfer.
#[utoipa::path(
    get,
    path = "/api/v1/admin/node-transfers/{id}",
    tag = "backups",
    operation_id = "adminGetNodeTransfer",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Transfer id")),
    responses(
        (status = 200, description = "The transfer", body = NodeTransferView),
        (status = 404, description = "No such transfer", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn show(
    State(state): State<BackupsState>,
    _who: Require<BackupsRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeTransferView>, AppError> {
    store::get(&state.db, id)
        .await
        .map_err(internal)?
        .map(view)
        .map(Json)
        .ok_or_else(|| AppError::not_found("transfer"))
}

/// Start moving an account to another node.
#[utoipa::path(
    post,
    path = "/api/v1/admin/node-transfers",
    tag = "backups",
    operation_id = "adminStartNodeTransfer",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = StartTransfer,
    responses(
        (status = 201, description = "Started", body = NodeTransferView),
        (status = 404, description = "No such account, node or destination", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`transfer_active`, `same_node`, `node_unavailable`, `tenant_not_active` or `transfer_unsupported` (the account has what a transfer cannot carry yet)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Json(req): Json<StartTransfer>,
) -> Result<(StatusCode, Json<NodeTransferView>), AppError> {
    let tenant = tenants::get(&state.db, req.tenant_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("account"))?;
    if tenant.status != builtpanel_domain::TenantStatus::Active {
        return Err(AppError::conflict(
            "tenant_not_active",
            "Only a running account can be moved.",
        ));
    }
    if tenant.node_id == req.to_node_id {
        return Err(AppError::conflict(
            "same_node",
            "The account is on that node already.",
        ));
    }
    let node_ok: Option<bool> = builtpanel_database::sqlx::query_scalar(
        "SELECT status = 'online' FROM nodes WHERE id = $1",
    )
    .bind(req.to_node_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;
    match node_ok {
        None => return Err(AppError::not_found("node")),
        Some(false) => {
            return Err(AppError::conflict(
                "node_unavailable",
                "The new node is not online.",
            ));
        }
        Some(true) => {}
    }
    let destination = match req.destination_id {
        Some(d) => backup_store::destination(&state.db, d)
            .await
            .map_err(internal)?
            .map(|d| d.id),
        None => backup_store::default_destination(&state.db)
            .await
            .map_err(internal)?,
    }
    .ok_or_else(|| AppError::not_found("backup destination"))?;
    let unsupported = store::unsupported(&state.db, tenant.id)
        .await
        .map_err(internal)?;
    if !unsupported.is_empty() {
        return Err(AppError::conflict(
            "transfer_unsupported",
            format!(
                "A transfer cannot carry these yet: {}.",
                unsupported.join(", ")
            ),
        ));
    }
    let made = store::create(
        &state.db,
        tenant.id,
        req.to_node_id,
        destination,
        req.auto_cutover,
        Some(who.0.account_id()),
    )
    .await
    .map_err(|e| match e {
        CreateError::Busy => {
            AppError::conflict("transfer_active", "This account is being moved already.")
        }
        CreateError::Database(e) => internal(e),
    })?;
    builtpanel_audit::insert(
        &state.db,
        &entry("node_transfer.started", &who.0, made.id).metadata(serde_json::json!({
            "tenant_id": tenant.id,
            "from_node": made.from_node,
            "to_node": made.to_node,
        })),
    )
    .await
    .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(view(made))))
}

async fn transition(
    state: &BackupsState,
    who: &Principal,
    id: Uuid,
    allowed: &[&str],
    to: &str,
    action: &'static str,
) -> Result<Json<NodeTransferView>, AppError> {
    let t = store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("transfer"))?;
    if !allowed.contains(&t.state.as_str()) {
        return Err(AppError::conflict(
            "wrong_state",
            format!("This cannot be done while the transfer is {}.", t.state),
        ));
    }
    match to {
        // The last pass: the loop takes it from here.
        "cutover" => {
            builtpanel_database::sqlx::query(
                "UPDATE tenant_transfers SET auto_cutover = true, updated_at = now() WHERE id = $1",
            )
            .bind(id)
            .execute(&state.db)
            .await
            .map_err(internal)?;
        }
        "cleaning" => {
            let tenant = tenants::get(&state.db, t.tenant_id)
                .await
                .map_err(internal)?
                .ok_or_else(|| AppError::not_found("account"))?;
            let op = queue(
                &state.db,
                t.from_node,
                t.tenant_id,
                &Operation::DeleteTenant {
                    tenant_id: t.tenant_id,
                    username: tenant.username,
                    uid: u32::try_from(tenant.uid).unwrap_or(0),
                    purge_data: true,
                },
                &format!("transfer:{}:clean", t.id.simple()),
            )
            .await
            .map_err(|e| internal(std::io::Error::other(e.to_string())))?;
            store::set_state(&state.db, id, "cleaning", Some(op), None)
                .await
                .map_err(internal)?;
        }
        "cancelling" => {
            // Before the switch: the old node keeps the account (and gets it back if it was
            // frozen), and what was made on the new node goes.
            let tenant = tenants::get(&state.db, t.tenant_id)
                .await
                .map_err(internal)?
                .ok_or_else(|| AppError::not_found("account"))?;
            if t.frozen_at.is_some() {
                queue(
                    &state.db,
                    t.from_node,
                    t.tenant_id,
                    &Operation::ResumeTenant {
                        username: tenant.username.clone(),
                    },
                    &format!("transfer:{}:resume-old", t.id.simple()),
                )
                .await
                .map_err(|e| internal(std::io::Error::other(e.to_string())))?;
            }
            let op = queue(
                &state.db,
                t.to_node,
                t.tenant_id,
                &Operation::DeleteTenant {
                    tenant_id: t.tenant_id,
                    username: tenant.username,
                    uid: u32::try_from(tenant.uid).unwrap_or(0),
                    purge_data: true,
                },
                &format!("transfer:{}:cancel", t.id.simple()),
            )
            .await
            .map_err(|e| internal(std::io::Error::other(e.to_string())))?;
            store::set_state(&state.db, id, "cancelling", Some(op), None)
                .await
                .map_err(internal)?;
        }
        other => {
            return Err(internal(std::io::Error::other(format!(
                "unknown transition {other}"
            ))));
        }
    }
    builtpanel_audit::insert(&state.db, &entry(action, who, id))
        .await
        .map_err(internal)?;
    store::get(&state.db, id)
        .await
        .map_err(internal)?
        .map(view)
        .map(Json)
        .ok_or_else(|| AppError::not_found("transfer"))
}

/// Go on to the last pass: the account is frozen on the old node, what changed is carried, and
/// the account is switched to the new node.
#[utoipa::path(
    post,
    path = "/api/v1/admin/node-transfers/{id}/cutover",
    tag = "backups",
    operation_id = "adminCutOverNodeTransfer",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Transfer id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Going on", body = NodeTransferView),
        (status = 404, description = "No such transfer", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`wrong_state`: the first copy is not there yet, or it is over", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn cutover(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeTransferView>, AppError> {
    transition(
        &state,
        &who.0,
        id,
        &["precopied"],
        "cutover",
        "node_transfer.cutover",
    )
    .await
}

/// Remove the account from the old node (after the switch).
#[utoipa::path(
    post,
    path = "/api/v1/admin/node-transfers/{id}/cleanup",
    tag = "backups",
    operation_id = "adminCleanUpNodeTransfer",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Transfer id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Being removed", body = NodeTransferView),
        (status = 404, description = "No such transfer", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`wrong_state`: the account has not been switched", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn cleanup(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeTransferView>, AppError> {
    transition(
        &state,
        &who.0,
        id,
        &["switched"],
        "cleaning",
        "node_transfer.cleaned_up",
    )
    .await
}

/// Give up before the switch: the old node keeps the account, the new node's copy is removed.
#[utoipa::path(
    post,
    path = "/api/v1/admin/node-transfers/{id}/cancel",
    tag = "backups",
    operation_id = "adminCancelNodeTransfer",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Transfer id"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "Being cancelled", body = NodeTransferView),
        (status = 404, description = "No such transfer", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`wrong_state`: it is switched or over", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn cancel(
    State(state): State<BackupsState>,
    who: Require<BackupsManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeTransferView>, AppError> {
    transition(
        &state,
        &who.0,
        id,
        &[
            "requested",
            "provisioning",
            "precopy_backup",
            "precopy_restore",
            "precopied",
            "failed",
        ],
        "cancelling",
        "node_transfer.cancelled",
    )
    .await
}

/// A message for a field that is wrong, used by tests.
#[must_use]
pub fn field_error(field: &'static str, message: &'static str) -> FieldError {
    FieldError::new(field, "invalid", message)
}
