//! An account's own services (T10.12): its containers, listed and restarted.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET | `/api/v1/client/services` | the owner |
//! | POST | `/api/v1/client/containers/{name}/restart` | the owner |
//! | GET | `/api/v1/client/processes` | the owner |
//! | POST | `/api/v1/client/processes/{container}/{pid}/signal` | the owner |
//! | GET | `/api/v1/admin/nodes/{id}/server` | `admin.server.read` |
//! | POST | `/api/v1/admin/nodes/{id}/server/{timezone,swap,drop-caches,reboot}` | `admin.server.manage`, sudo mode |
//! | GET | `/api/v1/admin/nodes/{id}/server/stats` | `admin.server.read` |
//! | GET | `/api/v1/admin/nodes/{id}/server/processes`, `.../processes/{pid}` | `admin.server.read` |
//! | POST | `/api/v1/admin/nodes/{id}/server/processes/{pid}/signal` | `admin.server.manage`, sudo mode |
//! | GET | `/api/v1/admin/nodes/{id}/services` | `admin.nodes.read` |
//! | POST | `/api/v1/admin/nodes/{id}/services/{name}/restart` | `admin.nodes.manage` |
//! | GET | `/api/v1/admin/nodes/{id}/services/{name}/log` | `admin.nodes.read` |
//!
//! The node's core services (`database`, `phpmyadmin`, `sftp`, `caddy`, `agent`) are
//! watched every half minute; one that stops is announced once as `node_service_down`
//! (and `node_service_recovered`), which the notification module (phase 18) turns into mail.
//! Only the first three can be restarted or have their log read; the web server and the agent
//! are host services (a restart of those needs the privileged helper: not part of this step).
//!
//! Only containers that carry the account's id are listed or restarted: the node, not this
//! module, decides that (`restart_container`), so another account's container, the node's
//! database or a name that does not exist all answer `404`.

use std::time::Duration;

use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    response::{IntoResponse as _, Response},
    routing::{get, post, put},
};
use builtpanel_database::{
    accounts::DbAccountKind,
    operations::{self, NewOperation, Status},
    sqlx::PgPool,
    tenants, users,
};
use builtpanel_http_kit::{AppError, ProblemDetails};
use builtpanel_node_protocol::{Operation, Output};
use builtpanel_rbac::{
    Principal, Require, RequireRecent,
    permissions::{NodesManage, NodesRead, ServerManage, ServerRead},
};
use builtpanel_session::SessionStore;
use serde::Serialize;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/services/module.toml");

/// How long a request waits for the node by default.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(20);

/// What the module's handlers share.
#[derive(Clone)]
pub struct ServicesState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// How long a request waits for the node before answering `202`.
    pub wait: Duration,
}

impl FromRef<ServicesState> for PgPool {
    fn from_ref(s: &ServicesState) -> Self {
        s.db.clone()
    }
}

impl FromRef<ServicesState> for SessionStore {
    fn from_ref(s: &ServicesState) -> Self {
        s.sessions.clone()
    }
}

/// Routes for administrators.
pub fn admin_router() -> Router<ServicesState> {
    Router::new()
        .route("/api/v1/admin/nodes/{id}/server", get(server_info))
        .route(
            "/api/v1/admin/nodes/{id}/server/timezone",
            post(set_timezone),
        )
        .route("/api/v1/admin/nodes/{id}/server/swap", post(set_swap))
        .route(
            "/api/v1/admin/nodes/{id}/server/drop-caches",
            post(drop_caches),
        )
        .route("/api/v1/admin/nodes/{id}/server/reboot", post(reboot_node))
        .route("/api/v1/admin/nodes/{id}/server/drop-swap", post(drop_swap))
        .route("/api/v1/admin/nodes/{id}/server/stats", get(server_stats))
        .route(
            "/api/v1/admin/nodes/{id}/server/processes",
            get(host_processes),
        )
        .route(
            "/api/v1/admin/nodes/{id}/server/processes/{pid}",
            get(host_process),
        )
        .route(
            "/api/v1/admin/nodes/{id}/server/processes/{pid}/signal",
            post(signal_host_process),
        )
        .route("/api/v1/admin/nodes/{id}/ssh", get(ssh_access))
        .route(
            "/api/v1/admin/nodes/{id}/ssh/settings",
            put(set_ssh_settings),
        )
        .route("/api/v1/admin/nodes/{id}/ssh/keys", post(add_ssh_key))
        .route(
            "/api/v1/admin/nodes/{id}/ssh/keys/remove",
            post(remove_ssh_key),
        )
        .route(
            "/api/v1/admin/nodes/{id}/ssh/config",
            put(write_sshd_config),
        )
        .route("/api/v1/admin/nodes/{id}/services", get(node_services))
        .route(
            "/api/v1/admin/nodes/{id}/services/{name}/restart",
            post(restart_node_service),
        )
        .route(
            "/api/v1/admin/nodes/{id}/services/{name}/log",
            get(node_service_log),
        )
}

/// Routes for hosting accounts.
pub fn client_router() -> Router<ServicesState> {
    Router::new()
        .route("/api/v1/client/services", get(list_services))
        .route(
            "/api/v1/client/containers/{name}/restart",
            post(restart_service),
        )
        .route("/api/v1/client/processes", get(list_processes))
        .route(
            "/api/v1/client/processes/{container}/{pid}/signal",
            post(signal_process),
        )
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

/// One of the account's containers.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceView {
    /// Its name (`bp-<account>-<role>`).
    pub name: String,
    /// What it is for (`web`, `php`).
    pub role: String,
    /// The image it runs.
    pub image: String,
    /// `running`, `exited`, `restarting`, `created`, ...
    pub state: String,
    /// Docker's words ("Up 3 hours").
    pub status: String,
    /// When it was made (Unix seconds).
    pub created: i64,
}

/// The services.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceList {
    /// By name.
    pub items: Vec<ServiceView>,
}

/// A restarted service.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceRestarted {
    /// The container.
    pub name: String,
    /// Its state now.
    pub state: String,
}

/// The node did not answer in time.
#[derive(Debug, Serialize, ToSchema)]
pub struct Pending {
    /// The operation.
    pub operation_id: Uuid,
}

async fn tenant_of(state: &ServicesState, who: &Principal) -> Result<tenants::Tenant, AppError> {
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

/// Asks the node and waits; `None` when it did not answer in time.
async fn ask(
    state: &ServicesState,
    tenant: &tenants::Tenant,
    who: &Principal,
    op: &Operation,
) -> Result<Result<Output, Uuid>, AppError> {
    let payload = serde_json::to_value(op).map_err(internal)?;
    let queued = operations::enqueue(
        &state.db,
        &NewOperation {
            node_id: tenant.node_id,
            kind: op.type_name(),
            tenant_id: Some(tenant.id),
            payload: &payload,
            idempotency_key: &format!("service:{}", Uuid::new_v4().simple()),
            created_by: Some(who.account_id()),
            ttl_secs: 120,
            max_attempts: 1,
        },
    )
    .await
    .map_err(|e| match e {
        operations::EnqueueError::UnknownNode => AppError::conflict(
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
        match row.status {
            Status::Succeeded => {
                let output: Output = row
                    .output
                    .and_then(|o| serde_json::from_value(o).ok())
                    .ok_or_else(|| internal(std::io::Error::other("an unreadable answer")))?;
                return Ok(Ok(output));
            }
            Status::Failed => {
                let message = row
                    .error
                    .as_ref()
                    .and_then(|e| e.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                return Err(if message.contains("no such container") {
                    AppError::not_found("service")
                } else {
                    AppError::unavailable(
                        "node_unavailable",
                        "The node could not do this; try again in a moment.",
                    )
                });
            }
            Status::Pending | Status::Dispatched | Status::Retryable => {}
        }
        if started.elapsed() >= state.wait {
            return Ok(Err(id));
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

/// The account's containers.
#[utoipa::path(
    get,
    path = "/api/v1/client/services",
    tag = "services",
    operation_id = "clientListServices",
    security(("session" = [])),
    responses(
        (status = 200, description = "The services", body = ServiceList),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`node_unavailable`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_services(
    State(state): State<ServicesState>,
    who: Principal,
) -> Result<Response, AppError> {
    let tenant = tenant_of(&state, &who).await?;
    let op = Operation::ListContainers {
        tenant_id: tenant.id,
    };
    match ask(&state, &tenant, &who, &op).await? {
        Ok(Output::Containers { items }) => Ok(Json(ServiceList {
            items: items
                .into_iter()
                .map(|c| ServiceView {
                    name: c.name,
                    role: c.role,
                    image: c.image,
                    state: c.state,
                    status: c.status,
                    created: c.created,
                })
                .collect(),
        })
        .into_response()),
        Ok(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response())
        }
    }
}

/// Restart one of the account's services.
#[utoipa::path(
    post,
    path = "/api/v1/client/containers/{name}/restart",
    tag = "services",
    operation_id = "clientRestartContainer",
    security(("session" = [])),
    params(
        ("name" = String, Path, description = "The container's name"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "ServiceRestarted", body = ServiceRestarted),
        (status = 202, description = "The node is slow; the restart goes on", body = Pending),
        (status = 404, description = "The account has no such service", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`node_unavailable`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn restart_service(
    State(state): State<ServicesState>,
    who: Principal,
    Path(name): Path<String>,
) -> Result<Response, AppError> {
    let tenant = tenant_of(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can restart its services.",
        ));
    }
    let op = Operation::RestartContainer {
        tenant_id: tenant.id,
        name: name.clone(),
    };
    match ask(&state, &tenant, &who, &op).await? {
        Ok(Output::ContainerRestarted { name, state: s }) => {
            builtpanel_audit_entry(&state, &who, &tenant, &name).await?;
            Ok(Json(ServiceRestarted { name, state: s }).into_response())
        }
        Ok(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response())
        }
    }
}

async fn builtpanel_audit_entry(
    state: &ServicesState,
    who: &Principal,
    tenant: &tenants::Tenant,
    name: &str,
) -> Result<(), AppError> {
    sqlx_insert_audit(&state.db, who, tenant, name)
        .await
        .map_err(internal)
}

async fn sqlx_insert_audit(
    db: &PgPool,
    who: &Principal,
    tenant: &tenants::Tenant,
    name: &str,
) -> Result<(), builtpanel_database::sqlx::Error> {
    builtpanel_audit::insert(
        db,
        &builtpanel_audit::AuditEntry::new("service.restarted", builtpanel_audit::Outcome::Success)
            .actor(who.account_id())
            .tenant(tenant.id)
            .metadata(serde_json::json!({ "container": name })),
    )
    .await
}

/// A process inside one of the account's containers.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProcessView {
    /// The container it runs in.
    pub container: String,
    /// Its id inside the container.
    pub pid: u32,
    /// The user id it runs as.
    pub uid: u32,
    /// `R` running, `S` sleeping, `D` waiting for the disk, `Z` zombie, ...
    pub state: String,
    /// The command line, clipped.
    pub command: String,
}

/// The processes.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProcessList {
    /// By container and id.
    pub items: Vec<ProcessView>,
}

/// The processes inside the account's containers (and only its own).
#[utoipa::path(
    get,
    path = "/api/v1/client/processes",
    tag = "services",
    operation_id = "clientListProcesses",
    security(("session" = [])),
    responses(
        (status = 200, description = "The processes", body = ProcessList),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 403, description = "`client_only`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list_processes(
    State(state): State<ServicesState>,
    who: Principal,
) -> Result<Response, AppError> {
    let tenant = tenant_of(&state, &who).await?;
    let op = Operation::ListProcesses {
        tenant_id: tenant.id,
    };
    match ask(&state, &tenant, &who, &op).await? {
        Ok(Output::Processes { items }) => Ok(Json(ProcessList {
            items: items
                .into_iter()
                .map(|p| ProcessView {
                    container: p.container,
                    pid: p.pid,
                    uid: p.uid,
                    state: p.state,
                    command: p.command,
                })
                .collect(),
        })
        .into_response()),
        Ok(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response())
        }
    }
}

/// What to send.
#[derive(Debug, serde::Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SignalRequest {
    /// `term` (ask it to end; default) or `kill` (end it at once).
    #[serde(default = "term")]
    pub signal: String,
    /// After `term`: wait up to ten seconds and `kill` what is still there.
    #[serde(default)]
    pub escalate: bool,
}

fn term() -> String {
    "term".to_owned()
}

/// How it went.
#[derive(Debug, Serialize, ToSchema)]
pub struct Signalled {
    /// The process.
    pub pid: u32,
    /// `exited`, `killed`, `not_found` or `still_running`.
    pub outcome: String,
}

/// End a process of the account: SIGTERM, or SIGKILL; with `escalate`, SIGTERM and then
/// SIGKILL after ten seconds if it stays. Process 1 of a container cannot be signalled
/// (restart the service instead).
#[utoipa::path(
    post,
    path = "/api/v1/client/processes/{container}/{pid}/signal",
    tag = "services",
    operation_id = "clientSignalProcess",
    security(("session" = [])),
    params(
        ("container" = String, Path, description = "The container's name"),
        ("pid" = u32, Path, description = "The process id inside it (2 or more)"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = SignalRequest,
    responses(
        (status = 200, description = "Done", body = Signalled),
        (status = 202, description = "The node is slow; the signal goes on", body = Pending),
        (status = 404, description = "The account has no such container", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`tenant_not_running`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A signal that is not `term` or `kill`, or process 1", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn signal_process(
    State(state): State<ServicesState>,
    who: Principal,
    Path((container, pid)): Path<(String, u32)>,
    Json(req): Json<SignalRequest>,
) -> Result<Response, AppError> {
    let tenant = tenant_of(&state, &who).await?;
    if tenant.status.as_str() != "active" {
        return Err(AppError::conflict(
            "tenant_not_running",
            "Only a running account can end processes.",
        ));
    }
    let kill = match req.signal.as_str() {
        "term" => false,
        "kill" => true,
        _ => {
            return Err(AppError::validation(vec![
                builtpanel_http_kit::FieldError::new("signal", "invalid", "term or kill."),
            ]));
        }
    };
    if pid < 2 {
        return Err(AppError::validation(vec![
            builtpanel_http_kit::FieldError::new(
                "pid",
                "invalid",
                "Process 1 cannot be ended: restart the service instead.",
            ),
        ]));
    }
    let op = Operation::SignalProcess {
        tenant_id: tenant.id,
        container: container.clone(),
        pid,
        kill,
        escalate: req.escalate && !kill,
    };
    match ask(&state, &tenant, &who, &op).await? {
        Ok(Output::ProcessSignalled { pid, outcome }) => {
            builtpanel_audit::insert(
                &state.db,
                &builtpanel_audit::AuditEntry::new(
                    "process.signalled",
                    builtpanel_audit::Outcome::Success,
                )
                .actor(who.account_id())
                .tenant(tenant.id)
                .metadata(serde_json::json!({
                    "container": container, "pid": pid, "signal": req.signal
                })),
            )
            .await
            .map_err(internal)?;
            let outcome = match outcome {
                builtpanel_node_protocol::SignalOutcome::Exited => "exited",
                builtpanel_node_protocol::SignalOutcome::Killed => "killed",
                builtpanel_node_protocol::SignalOutcome::NotFound => "not_found",
                builtpanel_node_protocol::SignalOutcome::StillRunning => "still_running",
            };
            Ok(Json(Signalled {
                pid,
                outcome: outcome.to_owned(),
            })
            .into_response())
        }
        Ok(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response())
        }
    }
}

/// What a node says about itself.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerInfoView {
    /// The machine's name.
    pub hostname: String,
    /// The distribution.
    pub os: String,
    /// The kernel release.
    pub kernel: String,
    /// The time zone (`Area/City`).
    pub timezone: String,
    /// Seconds since boot.
    pub uptime_seconds: u64,
    /// Memory of the machine, in bytes.
    pub memory_total_bytes: u64,
    /// Memory programs could still use, in bytes.
    pub memory_available_bytes: u64,
    /// Swap in all, in bytes.
    pub swap_total_bytes: u64,
    /// Swap in use, in bytes.
    pub swap_used_bytes: u64,
    /// The SSH server's safety settings as it will use them (name, value); read-only.
    pub ssh: Vec<SshSetting>,
}

/// One SSH server setting.
#[derive(Debug, Serialize, ToSchema)]
pub struct SshSetting {
    /// The setting (lower case).
    pub name: String,
    /// Its value.
    pub value: String,
}

/// The node's system, time zone, memory, swap and SSH settings.
#[utoipa::path(
    get,
    path = "/api/v1/admin/nodes/{id}/server",
    tag = "services",
    operation_id = "adminServerInfo",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id")),
    responses(
        (status = 200, description = "The node", body = ServerInfoView),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 403, description = "`permission_denied` (needs admin.server.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn server_info(
    State(state): State<ServicesState>,
    who: Require<ServerRead>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    match ask_node(&state, id, &who.0, &Operation::ReportServerInfo).await? {
        Ok(Output::ServerInfo { info }) => Ok(Json(ServerInfoView {
            hostname: info.hostname,
            os: info.os,
            kernel: info.kernel,
            timezone: info.timezone,
            uptime_seconds: info.uptime_seconds,
            memory_total_bytes: info.memory_total_bytes,
            memory_available_bytes: info.memory_available_bytes,
            swap_total_bytes: info.swap_total_bytes,
            swap_used_bytes: info.swap_used_bytes,
            ssh: info
                .ssh
                .into_iter()
                .map(|(name, value)| SshSetting { name, value })
                .collect(),
        })
        .into_response()),
        Ok(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response())
        }
    }
}

/// Which time zone.
#[derive(Debug, serde::Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TimezoneRequest {
    /// `Area/City`, such as `Asia/Dhaka` or `UTC`.
    pub zone: String,
}

/// How big the swap file is to be.
#[derive(Debug, serde::Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SwapRequest {
    /// MiB, 0 to 32768; 0 takes the swap file away.
    pub megabytes: u32,
}

/// A reboot has to be confirmed with the node's name.
#[derive(Debug, serde::Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RebootRequest {
    /// The node's name, typed as it is.
    pub confirm: String,
    /// Restart at once, without waiting for the services to stop (default: wait).
    #[serde(default)]
    pub hard: bool,
}

/// Done.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerActionView {
    /// Which action.
    pub action: String,
}

async fn server_action(
    state: &ServicesState,
    who: &Principal,
    id: Uuid,
    op: Operation,
    audit_action: &'static str,
    detail: serde_json::Value,
) -> Result<Response, AppError> {
    match ask_node(state, id, who, &op).await? {
        Ok(Output::ServerActionDone { action }) => {
            builtpanel_audit::insert(
                &state.db,
                &builtpanel_audit::AuditEntry::new(
                    audit_action,
                    builtpanel_audit::Outcome::Success,
                )
                .actor(who.account_id())
                .target("node", id)
                .metadata(detail),
            )
            .await
            .map_err(internal)?;
            Ok(Json(ServerActionView { action }).into_response())
        }
        Ok(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response())
        }
    }
}

// ------------------------------------------------------------------- live numbers and processes

/// A live reading of a node. Counters only grow: work rates out from two readings.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerStatsView {
    /// When it was taken (milliseconds since 1970).
    pub taken_at_ms: u64,
    /// Load average over 1, 5 and 15 minutes.
    pub load: Vec<f64>,
    /// Busy share of all processors, 0 to 100.
    pub cpu_percent: f64,
    /// The same per processor.
    pub cpu_cores_percent: Vec<f64>,
    /// Memory in all, in bytes.
    pub memory_total_bytes: u64,
    /// Memory in use (not counting cache), in bytes.
    pub memory_used_bytes: u64,
    /// Swap in all, in bytes.
    pub swap_total_bytes: u64,
    /// Swap in use, in bytes.
    pub swap_used_bytes: u64,
    /// The root file system.
    pub disk: DiskUsageView,
    /// Every disk with its counters since boot.
    pub disk_io: Vec<DiskIoView>,
    /// Every network interface with its counters since boot.
    pub net_io: Vec<NetIoView>,
}

/// Space on the root file system.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiskUsageView {
    /// The device.
    pub device: String,
    /// The file system type.
    pub filesystem: String,
    /// Size in bytes.
    pub total_bytes: u64,
    /// Used, in bytes.
    pub used_bytes: u64,
    /// Free, in bytes.
    pub free_bytes: u64,
}

/// Counters of one disk.
#[derive(Debug, Serialize, ToSchema)]
pub struct DiskIoView {
    /// The name.
    pub name: String,
    /// Reads completed.
    pub read_count: u64,
    /// Writes completed.
    pub write_count: u64,
    /// Bytes read.
    pub read_bytes: u64,
    /// Bytes written.
    pub write_bytes: u64,
    /// Milliseconds spent reading.
    pub read_ms: u64,
    /// Milliseconds spent writing.
    pub write_ms: u64,
}

/// Counters of one network interface.
#[derive(Debug, Serialize, ToSchema)]
pub struct NetIoView {
    /// The name.
    pub name: String,
    /// Bytes sent.
    pub bytes_sent: u64,
    /// Bytes received.
    pub bytes_received: u64,
    /// Packets sent.
    pub packets_sent: u64,
    /// Packets received.
    pub packets_received: u64,
    /// Receive errors.
    pub errors_in: u64,
    /// Send errors.
    pub errors_out: u64,
}

/// One process of the node.
#[derive(Debug, Serialize, ToSchema)]
pub struct HostProcessView {
    /// Process id.
    pub pid: u32,
    /// Its parent.
    pub ppid: u32,
    /// The user it runs as.
    pub owner: String,
    /// The user id.
    pub uid: u32,
    /// Its nice value (priority; lower is more important).
    pub priority: i32,
    /// `R`, `S`, `D`, `Z`, `T`.
    pub state: String,
    /// Share of one processor, in percent.
    pub cpu_percent: f64,
    /// Share of the node's memory, in percent.
    pub memory_percent: f64,
    /// Resident memory, in bytes.
    pub memory_bytes: u64,
    /// The command line, clipped.
    pub command: String,
}

/// What can be read about one process.
#[derive(Debug, Serialize, ToSchema)]
pub struct HostProcessDetailView {
    /// Process id.
    pub pid: u32,
    /// Its parent.
    pub ppid: u32,
    /// The user it runs as.
    pub owner: String,
    /// Its name.
    pub name: String,
    /// `R`, `S`, `D`, `Z`, `T`.
    pub state: String,
    /// Threads.
    pub threads: u32,
    /// Resident memory, in bytes.
    pub memory_bytes: u64,
    /// Seconds since it started.
    pub age_seconds: u64,
    /// Its nice value.
    pub priority: i32,
    /// The whole command line.
    pub command_line: String,
    /// Its control groups, one per line.
    pub cgroup: String,
}

/// Which signal.
#[derive(Debug, serde::Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HostSignalRequest {
    /// `term` (ask it to end, and kill it after ten seconds if it does not) or `kill` (at once).
    pub signal: String,
}

/// What came of the signal.
#[derive(Debug, Serialize, ToSchema)]
pub struct HostSignalled {
    /// The process.
    pub pid: u32,
    /// `exited`, `killed`, `not_found` or `still_running`.
    pub outcome: String,
}

fn answer<T>(
    got: Result<Output, Uuid>,
    pick: impl FnOnce(Output) -> Option<T>,
) -> Result<Result<T, Uuid>, AppError> {
    match got {
        Ok(out) => pick(out)
            .map(Ok)
            .ok_or_else(|| internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => Ok(Err(operation_id)),
    }
}

fn pending(operation_id: Uuid) -> Response {
    (StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response()
}

/// A live reading of the node: load, processors, memory, swap, disk, disk and network counters.
#[utoipa::path(
    get,
    path = "/api/v1/admin/nodes/{id}/server/stats",
    tag = "services",
    operation_id = "adminServerStats",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id")),
    responses(
        (status = 200, description = "The reading", body = ServerStatsView),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 403, description = "`permission_denied` (needs admin.server.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn server_stats(
    State(state): State<ServicesState>,
    who: Require<ServerRead>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    let got = ask_node(&state, id, &who.0, &Operation::ReportServerStats).await?;
    Ok(
        match answer(got, |o| match o {
            Output::ServerStats { stats } => Some(stats),
            _ => None,
        })? {
            Ok(s) => Json(ServerStatsView {
                taken_at_ms: s.taken_at_ms,
                load: s.load_x100.iter().map(|v| f64::from(*v) / 100.0).collect(),
                cpu_percent: f64::from(s.cpu_percent_x100) / 100.0,
                cpu_cores_percent: s
                    .cpu_cores_percent_x100
                    .iter()
                    .map(|v| f64::from(*v) / 100.0)
                    .collect(),
                memory_total_bytes: s.memory_total_bytes,
                memory_used_bytes: s.memory_used_bytes,
                swap_total_bytes: s.swap_total_bytes,
                swap_used_bytes: s.swap_used_bytes,
                disk: DiskUsageView {
                    device: s.disk.device,
                    filesystem: s.disk.filesystem,
                    total_bytes: s.disk.total_bytes,
                    used_bytes: s.disk.used_bytes,
                    free_bytes: s.disk.free_bytes,
                },
                disk_io: s
                    .disk_io
                    .into_iter()
                    .map(|d| DiskIoView {
                        name: d.name,
                        read_count: d.read_count,
                        write_count: d.write_count,
                        read_bytes: d.read_bytes,
                        write_bytes: d.write_bytes,
                        read_ms: d.read_ms,
                        write_ms: d.write_ms,
                    })
                    .collect(),
                net_io: s
                    .net_io
                    .into_iter()
                    .map(|n| NetIoView {
                        name: n.name,
                        bytes_sent: n.bytes_sent,
                        bytes_received: n.bytes_received,
                        packets_sent: n.packets_sent,
                        packets_received: n.packets_received,
                        errors_in: n.errors_in,
                        errors_out: n.errors_out,
                    })
                    .collect(),
            })
            .into_response(),
            Err(operation_id) => pending(operation_id),
        },
    )
}

/// Every process of the node with its processor and memory use, the busiest first.
#[utoipa::path(
    get,
    path = "/api/v1/admin/nodes/{id}/server/processes",
    tag = "services",
    operation_id = "adminHostProcesses",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id")),
    responses(
        (status = 200, description = "The processes (at most 2000)", body = Vec<HostProcessView>),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 403, description = "`permission_denied` (needs admin.server.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn host_processes(
    State(state): State<ServicesState>,
    who: Require<ServerRead>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    let got = ask_node(&state, id, &who.0, &Operation::ListHostProcesses).await?;
    Ok(
        match answer(got, |o| match o {
            Output::HostProcesses { items } => Some(items),
            _ => None,
        })? {
            Ok(items) => Json(
                items
                    .into_iter()
                    .map(|p| HostProcessView {
                        pid: p.pid,
                        ppid: p.ppid,
                        owner: p.owner,
                        uid: p.uid,
                        priority: p.priority,
                        state: p.state,
                        cpu_percent: f64::from(p.cpu_percent_x100) / 100.0,
                        memory_percent: f64::from(p.memory_percent_x100) / 100.0,
                        memory_bytes: p.memory_bytes,
                        command: p.command,
                    })
                    .collect::<Vec<_>>(),
            )
            .into_response(),
            Err(operation_id) => pending(operation_id),
        },
    )
}

/// What can be read about one process of the node.
#[utoipa::path(
    get,
    path = "/api/v1/admin/nodes/{id}/server/processes/{pid}",
    tag = "services",
    operation_id = "adminHostProcess",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("pid" = u32, Path, description = "Process id")),
    responses(
        (status = 200, description = "The process", body = HostProcessDetailView),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 403, description = "`permission_denied` (needs admin.server.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "There is no such process", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn host_process(
    State(state): State<ServicesState>,
    who: Require<ServerRead>,
    Path((id, pid)): Path<(Uuid, u32)>,
) -> Result<Response, AppError> {
    let got = ask_node(&state, id, &who.0, &Operation::InspectHostProcess { pid }).await?;
    Ok(
        match answer(got, |o| match o {
            Output::HostProcessDetail { detail } => Some(detail),
            _ => None,
        })? {
            Ok(d) => Json(HostProcessDetailView {
                pid: d.pid,
                ppid: d.ppid,
                owner: d.owner,
                name: d.name,
                state: d.state,
                threads: d.threads,
                memory_bytes: d.memory_bytes,
                age_seconds: d.age_seconds,
                priority: d.priority,
                command_line: d.command_line,
                cgroup: d.cgroup,
            })
            .into_response(),
            Err(operation_id) => pending(operation_id),
        },
    )
}

/// End a process of the node. The first process, kernel threads and the panel's own programs are refused.
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/server/processes/{pid}/signal",
    tag = "services",
    operation_id = "adminSignalHostProcess",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("pid" = u32, Path, description = "Process id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = HostSignalRequest,
    responses(
        (status = 200, description = "What came of it", body = HostSignalled),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A signal that is not `term` or `kill`, process 1, or a process that may not be signalled", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn signal_host_process(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path((id, pid)): Path<(Uuid, u32)>,
    Json(req): Json<HostSignalRequest>,
) -> Result<Response, AppError> {
    let signal = match req.signal.as_str() {
        "term" => builtpanel_node_protocol::HostSignal::Terminate,
        "kill" => builtpanel_node_protocol::HostSignal::Kill,
        _ => {
            return Err(AppError::validation(vec![
                builtpanel_http_kit::FieldError::new("signal", "invalid", "term or kill."),
            ]));
        }
    };
    if pid < 2 {
        return Err(AppError::validation(vec![
            builtpanel_http_kit::FieldError::new("pid", "invalid", "Process 1 cannot be ended."),
        ]));
    }
    let got = ask_node(
        &state,
        id,
        &who.0,
        &Operation::SignalHostProcess { pid, signal },
    )
    .await?;
    match answer(got, |o| match o {
        Output::ProcessSignalled { pid, outcome } => Some((pid, outcome)),
        _ => None,
    })? {
        Ok((pid, outcome)) => {
            builtpanel_audit::insert(
                &state.db,
                &builtpanel_audit::AuditEntry::new(
                    "node_server.process_signalled",
                    builtpanel_audit::Outcome::Success,
                )
                .actor(who.0.account_id())
                .target("node", id)
                .metadata(serde_json::json!({ "pid": pid, "signal": req.signal })),
            )
            .await
            .map_err(internal)?;
            let outcome = match outcome {
                builtpanel_node_protocol::SignalOutcome::Exited => "exited",
                builtpanel_node_protocol::SignalOutcome::Killed => "killed",
                builtpanel_node_protocol::SignalOutcome::NotFound => "not_found",
                builtpanel_node_protocol::SignalOutcome::StillRunning => "still_running",
            };
            Ok(Json(HostSignalled {
                pid,
                outcome: outcome.to_owned(),
            })
            .into_response())
        }
        Err(operation_id) => Ok(pending(operation_id)),
    }
}

/// Set the node's time zone.
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/server/timezone",
    tag = "services",
    operation_id = "adminSetTimezone",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = TimezoneRequest,
    responses(
        (status = 200, description = "Done", body = ServerActionView),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Not a time zone of the node", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn set_timezone(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<TimezoneRequest>,
) -> Result<Response, AppError> {
    let zone = req.zone.trim().to_owned();
    server_action(
        &state,
        &who.0,
        id,
        Operation::SetTimezone { zone: zone.clone() },
        "node_server.timezone_set",
        serde_json::json!({ "zone": zone }),
    )
    .await
}

/// Make the node's swap file (or take it away with 0).
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/server/swap",
    tag = "services",
    operation_id = "adminSetSwap",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SwapRequest,
    responses(
        (status = 200, description = "Done", body = ServerActionView),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A size out of range", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn set_swap(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SwapRequest>,
) -> Result<Response, AppError> {
    if req.megabytes > 32_768 {
        return Err(AppError::validation(vec![
            builtpanel_http_kit::FieldError::new(
                "megabytes",
                "out_of_range",
                "Between 0 and 32768.",
            ),
        ]));
    }
    server_action(
        &state,
        &who.0,
        id,
        Operation::SetSwap {
            megabytes: req.megabytes,
        },
        "node_server.swap_set",
        serde_json::json!({ "megabytes": req.megabytes }),
    )
    .await
}

/// Drop the node's page cache.
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/server/drop-caches",
    tag = "services",
    operation_id = "adminDropCaches",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 200, description = "Done", body = ServerActionView),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn drop_caches(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    server_action(
        &state,
        &who.0,
        id,
        Operation::DropCaches,
        "node_server.caches_dropped",
        serde_json::json!({}),
    )
    .await
}

/// Move what is swapped out back into memory and turn swap on again. Refused when there is not
/// enough free memory for it.
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/server/drop-swap",
    tag = "services",
    operation_id = "adminDropSwap",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    responses(
        (status = 200, description = "Done", body = ServerActionView),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "There is not enough free memory for what is in swap", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn drop_swap(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    server_action(
        &state,
        &who.0,
        id,
        Operation::DropSwap,
        "node_server.swap_dropped",
        serde_json::json!({}),
    )
    .await
}

/// Reboot the node. The body must carry the node's name as it is.
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/server/reboot",
    tag = "services",
    operation_id = "adminRebootNode",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = RebootRequest,
    responses(
        (status = 200, description = "The reboot starts in a few seconds", body = ServerActionView),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "`confirm` is not the node's name", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn reboot_node(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<RebootRequest>,
) -> Result<Response, AppError> {
    let name: Option<String> =
        builtpanel_database::sqlx::query_scalar("SELECT name FROM nodes WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .map_err(internal)?;
    let name = name.ok_or_else(|| AppError::not_found("node"))?;
    if req.confirm != name {
        return Err(AppError::validation(vec![
            builtpanel_http_kit::FieldError::new(
                "confirm",
                "mismatch",
                "Type the node's name to confirm the reboot.",
            ),
        ]));
    }
    server_action(
        &state,
        &who.0,
        id,
        Operation::RebootNode { hard: req.hard },
        "node_server.reboot",
        serde_json::json!({ "node": name, "hard": req.hard }),
    )
    .await
}

/// A key that may log in as root over SSH.
#[derive(Debug, Serialize, ToSchema)]
pub struct SshKeyView {
    /// `ssh-ed25519`, `ssh-rsa`, ...
    pub kind: String,
    /// `SHA256:...`, as `ssh-keygen -l` prints it.
    pub fingerprint: String,
    /// The comment of the key line.
    pub comment: String,
}

/// The SSH access of a node.
#[derive(Debug, Serialize, ToSchema)]
pub struct SshAccessView {
    /// Every port the SSH server listens on.
    pub ports: Vec<u16>,
    /// `yes`, `no`, `prohibit-password` or `forced-commands-only`.
    pub root_login: String,
    /// Whether passwords may be used to log in.
    pub password_authentication: bool,
    /// Whether keys may be used to log in.
    pub pubkey_authentication: bool,
    /// The keys that may log in as root.
    pub keys: Vec<SshKeyView>,
    /// The text of `sshd_config`.
    pub config: String,
}

/// The SSH access of a node: how it listens, who may log in, and its configuration.
#[utoipa::path(
    get,
    path = "/api/v1/admin/nodes/{id}/ssh",
    tag = "services",
    operation_id = "adminSshAccess",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id")),
    responses(
        (status = 200, description = "The SSH access", body = SshAccessView),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 403, description = "`permission_denied` (needs admin.server.read)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn ssh_access(
    State(state): State<ServicesState>,
    who: Require<ServerRead>,
    Path(id): Path<Uuid>,
) -> Result<Response, AppError> {
    match ask_node(&state, id, &who.0, &Operation::ReadSshAccess).await? {
        Ok(Output::SshAccess { access }) => Ok(Json(SshAccessView {
            ports: access.ports,
            root_login: access.root_login,
            password_authentication: access.password_authentication,
            pubkey_authentication: access.pubkey_authentication,
            keys: access
                .keys
                .into_iter()
                .map(|k| SshKeyView {
                    kind: k.kind,
                    fingerprint: k.fingerprint,
                    comment: k.comment,
                })
                .collect(),
            config: access.config,
        })
        .into_response()),
        Ok(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response())
        }
    }
}

/// The SSH settings to change; what is absent stays as it is.
#[derive(Debug, serde::Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SshSettingsRequest {
    /// The port: 22, or 1024 to 65535 (not one the panel uses).
    #[serde(default)]
    pub port: Option<u16>,
    /// `yes`, `no`, `prohibit-password` or `forced-commands-only`.
    #[serde(default)]
    pub root_login: Option<String>,
    /// Whether passwords may be used to log in.
    #[serde(default)]
    pub password_authentication: Option<bool>,
    /// Whether keys may be used to log in.
    #[serde(default)]
    pub pubkey_authentication: Option<bool>,
}

/// Change the node's SSH settings. The node refuses a change that would lock everyone out, checks
/// the result, and puts the old settings back when SSH does not come up.
#[utoipa::path(
    put,
    path = "/api/v1/admin/nodes/{id}/ssh/settings",
    tag = "services",
    operation_id = "adminSetSshSettings",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SshSettingsRequest,
    responses(
        (status = 200, description = "Done", body = ServerActionView),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Nothing to change, a value that is not allowed, or a change that would lock everyone out", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn set_ssh_settings(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SshSettingsRequest>,
) -> Result<Response, AppError> {
    let mut errors = Vec::new();
    if req.port.is_none()
        && req.root_login.is_none()
        && req.password_authentication.is_none()
        && req.pubkey_authentication.is_none()
    {
        errors.push(builtpanel_http_kit::FieldError::new(
            "port",
            "required",
            "Give at least one setting to change.",
        ));
    }
    if req.port.is_some_and(|p| p != 22 && p < 1024) {
        errors.push(builtpanel_http_kit::FieldError::new(
            "port",
            "out_of_range",
            "Use 22, or a port from 1024 to 65535.",
        ));
    }
    if req
        .root_login
        .as_deref()
        .is_some_and(|v| !["yes", "no", "prohibit-password", "forced-commands-only"].contains(&v))
    {
        errors.push(builtpanel_http_kit::FieldError::new(
            "root_login",
            "invalid",
            "Choose yes, no, prohibit-password or forced-commands-only.",
        ));
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let detail = serde_json::json!({
        "port": req.port,
        "root_login": req.root_login,
        "password_authentication": req.password_authentication,
        "pubkey_authentication": req.pubkey_authentication,
    });
    server_action(
        &state,
        &who.0,
        id,
        Operation::SetSshSettings {
            port: req.port,
            root_login: req.root_login,
            password_authentication: req.password_authentication,
            pubkey_authentication: req.pubkey_authentication,
        },
        "node_server.ssh_settings",
        detail,
    )
    .await
}

/// A public key to authorise for root.
#[derive(Debug, serde::Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SshKeyRequest {
    /// One public key line: `ssh-ed25519 AAAA... comment`.
    pub key: String,
}

/// Authorise a public key for root over SSH.
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/ssh/keys",
    tag = "services",
    operation_id = "adminAddSshKey",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SshKeyRequest,
    responses(
        (status = 200, description = "Done", body = ServerActionView),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Not a public key this server accepts, or it is there already", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn add_ssh_key(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SshKeyRequest>,
) -> Result<Response, AppError> {
    if req.key.trim().is_empty() || req.key.len() > 8192 || req.key.contains(['\n', '\r', '\0']) {
        return Err(AppError::validation(vec![
            builtpanel_http_kit::FieldError::new(
                "key",
                "invalid",
                "Give one public key line, such as ssh-ed25519 AAAA... name.",
            ),
        ]));
    }
    // The audit entry names the key by its type and comment, not by the key.
    let words: Vec<&str> = req.key.split_whitespace().collect();
    let detail = serde_json::json!({
        "kind": words.first(),
        "comment": words.get(2..).map(|c| c.join(" ")),
    });
    server_action(
        &state,
        &who.0,
        id,
        Operation::AddSshKey { key: req.key },
        "node_server.ssh_key_added",
        detail,
    )
    .await
}

/// Which key to take away.
#[derive(Debug, serde::Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SshKeyRemoveRequest {
    /// The key's fingerprint (`SHA256:...`) as listed.
    pub fingerprint: String,
}

/// Take an authorised key away. Refused when it is the last one and password login is off.
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/ssh/keys/remove",
    tag = "services",
    operation_id = "adminRemoveSshKey",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SshKeyRemoveRequest,
    responses(
        (status = 200, description = "Done", body = ServerActionView),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "No key with this fingerprint, or the removal would lock everyone out", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn remove_ssh_key(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SshKeyRemoveRequest>,
) -> Result<Response, AppError> {
    if !req.fingerprint.starts_with("SHA256:") || req.fingerprint.len() > 80 {
        return Err(AppError::validation(vec![
            builtpanel_http_kit::FieldError::new(
                "fingerprint",
                "invalid",
                "Give the fingerprint as listed (SHA256:...).",
            ),
        ]));
    }
    let detail = serde_json::json!({ "fingerprint": req.fingerprint });
    server_action(
        &state,
        &who.0,
        id,
        Operation::RemoveSshKey {
            fingerprint: req.fingerprint,
        },
        "node_server.ssh_key_removed",
        detail,
    )
    .await
}

/// The new text of `sshd_config`.
#[derive(Debug, serde::Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SshConfigRequest {
    /// The whole file, at most 64 KiB.
    pub text: String,
}

/// Replace `sshd_config`. The node checks it with `sshd -t` first, refuses a text that would lock
/// everyone out, and puts the old file back when SSH does not come up with it.
#[utoipa::path(
    put,
    path = "/api/v1/admin/nodes/{id}/ssh/config",
    tag = "services",
    operation_id = "adminWriteSshdConfig",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id"), ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = SshConfigRequest,
    responses(
        (status = 200, description = "Done", body = ServerActionView),
        (status = 202, description = "The node is slow; it goes on", body = Pending),
        (status = 403, description = "`reauth_required` (sudo mode), `permission_denied` (needs admin.server.manage) or `csrf_failed`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "The SSH server does not accept the text, or it would lock everyone out", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn write_sshd_config(
    State(state): State<ServicesState>,
    who: RequireRecent<ServerManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<SshConfigRequest>,
) -> Result<Response, AppError> {
    if req.text.trim().is_empty() || req.text.len() > 65_536 {
        return Err(AppError::validation(vec![
            builtpanel_http_kit::FieldError::new(
                "text",
                "invalid",
                "The configuration must not be empty or longer than 64 KiB.",
            ),
        ]));
    }
    // The text itself stays out of the audit log; its size and a hash say what was written.
    let detail = serde_json::json!({ "bytes": req.text.len() });
    server_action(
        &state,
        &who.0,
        id,
        Operation::WriteSshdConfig { text: req.text },
        "node_server.sshd_config_written",
        detail,
    )
    .await
}

/// A core service of a node.
#[derive(Debug, Serialize, ToSchema)]
pub struct NodeServiceView {
    /// `database`, `phpmyadmin`, `sftp`, `caddy` or `agent`.
    pub name: String,
    /// `running`, `stopped`, `unreachable` or `not_installed`.
    pub state: String,
    /// Docker's words, or what went wrong.
    pub detail: String,
    /// Since when it is in this state (Unix seconds).
    pub since: i64,
    /// When it was last looked at (Unix seconds). A node that is off shows an old time.
    pub checked_at: i64,
}

/// The services of a node.
#[derive(Debug, Serialize, ToSchema)]
pub struct NodeServiceList {
    /// By name.
    pub items: Vec<NodeServiceView>,
}

/// The core services of a node, as it last reported them.
#[utoipa::path(
    get,
    path = "/api/v1/admin/nodes/{id}/services",
    tag = "services",
    operation_id = "adminNodeServices",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Node id")),
    responses(
        (status = 200, description = "The services", body = NodeServiceList),
        (status = 403, description = "`permission_denied` (needs admin.nodes.read)", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn node_services(
    State(state): State<ServicesState>,
    _who: Require<NodesRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<NodeServiceList>, AppError> {
    let items = builtpanel_database::node_services::list(&state.db, id)
        .await
        .map_err(internal)?
        .into_iter()
        .map(|s| NodeServiceView {
            name: s.name,
            state: s.state,
            detail: s.detail,
            since: s.since,
            checked_at: s.checked_at,
        })
        .collect();
    Ok(Json(NodeServiceList { items }))
}

/// Asks a node (no account) and waits.
async fn ask_node(
    state: &ServicesState,
    node_id: Uuid,
    who: &Principal,
    op: &Operation,
) -> Result<Result<Output, Uuid>, AppError> {
    let payload = serde_json::to_value(op).map_err(internal)?;
    let queued = operations::enqueue(
        &state.db,
        &NewOperation {
            node_id,
            kind: op.type_name(),
            tenant_id: None,
            payload: &payload,
            idempotency_key: &format!("nodeservice:{}", Uuid::new_v4().simple()),
            created_by: Some(who.account_id()),
            ttl_secs: 120,
            max_attempts: 1,
        },
    )
    .await
    .map_err(|e| match e {
        operations::EnqueueError::UnknownNode => AppError::not_found("node"),
        other => internal(other),
    })?;
    let id = queued.operation().id;
    let started = std::time::Instant::now();
    loop {
        let row = operations::get(&state.db, id)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::not_found("operation"))?;
        match row.status {
            Status::Succeeded => {
                let output: Output = row
                    .output
                    .and_then(|o| serde_json::from_value(o).ok())
                    .ok_or_else(|| internal(std::io::Error::other("an unreadable answer")))?;
                return Ok(Ok(output));
            }
            Status::Failed => {
                let message = row
                    .error
                    .as_ref()
                    .and_then(|e| e.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let refused = row
                    .error
                    .as_ref()
                    .and_then(|e| e.get("code"))
                    .and_then(serde_json::Value::as_str)
                    == Some("invalid_arguments");
                return Err(
                    if message.contains("not a core service") || message.contains("not installed") {
                        AppError::not_found("service")
                    } else if refused {
                        // The node refused what was asked: its words say what is wrong.
                        AppError::validation(vec![builtpanel_http_kit::FieldError::new(
                            "value",
                            "refused_by_node",
                            &message,
                        )])
                    } else {
                        AppError::unavailable(
                            "node_unavailable",
                            "The node could not do this; try again in a moment.",
                        )
                    },
                );
            }
            Status::Pending | Status::Dispatched | Status::Retryable => {}
        }
        if started.elapsed() >= state.wait {
            return Ok(Err(id));
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

/// Restart a core service of a node (`database`, `phpmyadmin` or `sftp`).
#[utoipa::path(
    post,
    path = "/api/v1/admin/nodes/{id}/services/{name}/restart",
    tag = "services",
    operation_id = "adminRestartNodeService",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Node id"),
        ("name" = String, Path, description = "`database`, `phpmyadmin` or `sftp`"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    responses(
        (status = 200, description = "ServiceRestarted", body = ServiceRestarted),
        (status = 202, description = "The node is slow; the restart goes on", body = Pending),
        (status = 404, description = "No such node, service, or it is not installed there", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`node_unavailable`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn restart_node_service(
    State(state): State<ServicesState>,
    who: Require<NodesManage>,
    Path((id, name)): Path<(Uuid, String)>,
) -> Result<Response, AppError> {
    let op = Operation::RestartNodeService { name: name.clone() };
    match ask_node(&state, id, &who.0, &op).await? {
        Ok(Output::ContainerRestarted { name, state: s }) => {
            builtpanel_audit::insert(
                &state.db,
                &builtpanel_audit::AuditEntry::new(
                    "node_service.restarted",
                    builtpanel_audit::Outcome::Success,
                )
                .actor(who.0.account_id())
                .target("node", id)
                .metadata(serde_json::json!({ "service": name })),
            )
            .await
            .map_err(internal)?;
            Ok(Json(ServiceRestarted { name, state: s }).into_response())
        }
        Ok(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response())
        }
    }
}

/// Which log.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct ServiceLogQuery {
    /// How many of the newest lines (1-2000, default 200).
    pub lines: Option<u32>,
}

/// One line of a service's log.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceLogLine {
    /// When (Unix milliseconds).
    pub at_ms: i64,
    /// The line.
    pub text: String,
}

/// The end of a service's log.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServiceLog {
    /// Oldest first.
    pub lines: Vec<ServiceLogLine>,
    /// Older lines were left out.
    pub truncated: bool,
}

/// The end of the log of a core service of a node (`database`, `phpmyadmin` or `sftp`).
#[utoipa::path(
    get,
    path = "/api/v1/admin/nodes/{id}/services/{name}/log",
    tag = "services",
    operation_id = "adminNodeServiceLog",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Node id"),
        ("name" = String, Path, description = "`database`, `phpmyadmin` or `sftp`"),
        ServiceLogQuery
    ),
    responses(
        (status = 200, description = "The lines", body = ServiceLog),
        (status = 202, description = "The node is slow; ask again", body = Pending),
        (status = 404, description = "No such node or service", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "A count out of range", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn node_service_log(
    State(state): State<ServicesState>,
    who: Require<NodesRead>,
    Path((id, name)): Path<(Uuid, String)>,
    axum::extract::Query(q): axum::extract::Query<ServiceLogQuery>,
) -> Result<Response, AppError> {
    let lines = q.lines.unwrap_or(200);
    if !(1..=builtpanel_node_protocol::MAX_LOG_LINES).contains(&lines) {
        return Err(AppError::validation(vec![
            builtpanel_http_kit::FieldError::new(
                "lines",
                "out_of_range",
                "Between 1 and 2000 lines.",
            ),
        ]));
    }
    let op = Operation::ReadServiceLog { name, tail: lines };
    match ask_node(&state, id, &who.0, &op).await? {
        Ok(Output::LogLines { lines, truncated }) => Ok(Json(ServiceLog {
            lines: lines
                .into_iter()
                .map(|l| ServiceLogLine {
                    at_ms: l.at_ms,
                    text: l.text,
                })
                .collect(),
            truncated,
        })
        .into_response()),
        Ok(_) => Err(internal(std::io::Error::other("an unexpected answer"))),
        Err(operation_id) => {
            Ok((StatusCode::ACCEPTED, Json(Pending { operation_id })).into_response())
        }
    }
}

/// The module's OpenAPI document.
#[derive(OpenApi)]
#[openapi(
    paths(
        list_services, restart_service, list_processes, signal_process, server_info,
        set_timezone, set_swap, drop_caches, drop_swap, server_stats, host_processes, host_process, signal_host_process, reboot_node, node_services, restart_node_service, node_service_log,
        ssh_access, set_ssh_settings, add_ssh_key, remove_ssh_key, write_sshd_config
    ),
    components(schemas(
        ServiceView, ServiceList, ServiceRestarted, Pending, ProcessView, ProcessList, SignalRequest,
        Signalled, ServerInfoView, SshSetting, TimezoneRequest, SwapRequest, RebootRequest,
        ServerActionView, NodeServiceView, NodeServiceList,
        ServiceLogLine, ServiceLog, SshKeyView, SshAccessView, SshSettingsRequest, SshKeyRequest,
        SshKeyRemoveRequest, SshConfigRequest, ServerStatsView, DiskUsageView, DiskIoView, NetIoView,
        HostProcessView, HostProcessDetailView, HostSignalRequest, HostSignalled
    )),
    tags((name = "services", description = "An account's own services"))
)]
pub struct ServicesApi;
