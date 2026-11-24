//! Moving a whole server (System, Migrate): the panel logs in to a new server over SSH, installs a node
//! there (the same one-command installer a blank server would run by hand) and joins it to this panel.
//! The accounts are then moved to it with the node transfers.
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | POST | `/api/v1/admin/system/migrate/probe` | `admin.nodes.manage`, sudo mode |
//! | POST | `/api/v1/admin/system/migrate/check` | same |
//! | GET, POST | `/api/v1/admin/system/migrate` | `admin.nodes.read`; `admin.nodes.manage`, sudo mode |
//! | GET | `/api/v1/admin/system/migrate/{id}` | `admin.nodes.read` |
//! | POST | `/api/v1/admin/system/migrate/{id}/move-accounts` | `admin.nodes.manage`, sudo mode |
//!
//! The server's SSH password is used for one connection and dropped: it is never stored, logged or put in
//! an audit entry, and the installer's output is stored with the password and the enrolment token removed.
//! The server's host key must be confirmed first (`probe` shows it; the other calls must name it), so a
//! password is only ever sent to the server the administrator saw. A job that is running when the panel
//! restarts is lost (it shows as failed): start it again.

use std::time::Duration;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    backups as backup_store, server_migrations as store,
    sqlx::{self},
    transfers::{self, CreateError},
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_mod_migration::remote::{self, Credential, FetchError, Source};
use builtpanel_rbac::permissions::{NodesManage, NodesRead};
use builtpanel_rbac::{Require, RequireRecent};
use builtpanel_session::rfc3339;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{NodesState, internal, issue_enrollment, valid_node_name};

/// Where the installer comes from, and whether the panel may connect to addresses of its own networks.
#[derive(Clone, Debug)]
pub struct MigrateSettings {
    /// The one-command installer (`get.sh`) a new server downloads.
    pub installer_url: String,
    /// Whether this machine's own addresses may be connected to (tests only).
    pub allow_local_ssh: bool,
}

impl Default for MigrateSettings {
    fn default() -> Self {
        Self {
            installer_url: DEFAULT_INSTALLER_URL.to_owned(),
            allow_local_ssh: false,
        }
    }
}

/// Where the one-command installer is published (the same release channel as the panel itself).
pub const DEFAULT_INSTALLER_URL: &str =
    "https://github.com/khairulalamruet-ai/DPanel/releases/latest/download/get.sh";

/// How long the new node may take to come online after the installer ended.
const JOIN_WAIT: Duration = Duration::from_secs(10 * 60);
/// Most jobs listed.
const LIST_LIMIT: i64 = 50;

fn default_port() -> u16 {
    22
}

fn default_user() -> String {
    "root".to_owned()
}

// ----------------------------------------------------------------------------------- requests

/// Which server to look at.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeRequest {
    /// The new server's address (a name or an IP address).
    #[schema(example = "203.0.113.10")]
    pub host: String,
    /// Its SSH port.
    #[serde(default = "default_port")]
    pub port: u16,
}

/// The key the server shows.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProbeView {
    /// The host key's SHA-256 fingerprint (`SHA256:...`). Check it against the server before going on.
    pub host_key_sha256: String,
}

/// How to log in to the new server. The password is used once and not kept.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// The new server's address.
    pub host: String,
    /// Its SSH port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// The login (`root`, or a user that can run `sudo` without a password).
    #[serde(default = "default_user")]
    pub user: String,
    /// The password.
    pub password: String,
    /// The fingerprint `probe` showed.
    pub host_key_sha256: String,
}

/// Start moving a server.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    /// The new server's address.
    pub host: String,
    /// Its SSH port.
    #[serde(default = "default_port")]
    pub port: u16,
    /// The login (`root`, or a user that can run `sudo` without a password).
    #[serde(default = "default_user")]
    pub user: String,
    /// The password.
    pub password: String,
    /// The fingerprint `probe` showed.
    pub host_key_sha256: String,
    /// The node's name in this panel: 2-63 characters, a-z, 0-9 and `-`, starting with a letter.
    #[schema(example = "new-server")]
    pub node_name: String,
}

/// What to move.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveRequest {
    /// The node whose accounts are moved (the old server).
    pub from_node_id: Uuid,
    /// Switch each account over as soon as its first copy is there (otherwise each waits for its cutover).
    #[serde(default)]
    pub auto_cutover: bool,
}

// ----------------------------------------------------------------------------------- views

/// What the new server is like.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerFacts {
    /// The operating system's name.
    pub os: String,
    /// `x86_64` or `aarch64`.
    pub arch: String,
    /// Processor cores.
    pub cpus: u32,
    /// Memory in MB.
    pub memory_mb: u64,
    /// Free space on `/` in GB.
    pub disk_free_gb: u64,
    /// Whether hardware virtualization is there (Kata Containers needs it).
    pub kvm: bool,
    /// Whether the login is root.
    pub root: bool,
    /// What stands in the way of a node here (empty when nothing does).
    pub problems: Vec<String>,
}

/// One job.
#[derive(Debug, Serialize, ToSchema)]
pub struct ServerMigrationView {
    /// Id.
    pub id: Uuid,
    /// The server's address.
    pub host: String,
    /// Its SSH port.
    pub port: i32,
    /// The login used.
    pub ssh_user: String,
    /// The node's name here.
    pub node_name: String,
    /// `running`, `completed` or `failed`.
    pub status: String,
    /// What it is doing: `connecting`, `installing`, `joining`, `joined`, or what went wrong.
    pub phase: String,
    /// The node, once it has joined.
    pub node_id: Option<Uuid>,
    /// The installer's output (the last 64 KB), secrets removed.
    pub log: String,
    /// Start (RFC 3339).
    #[schema(format = DateTime)]
    pub started_at: String,
    /// Last change (RFC 3339).
    #[schema(format = DateTime)]
    pub updated_at: String,
}

fn view(m: store::ServerMigration) -> ServerMigrationView {
    ServerMigrationView {
        id: m.id,
        host: m.host,
        port: m.port,
        ssh_user: m.ssh_user,
        node_name: m.node_name,
        status: m.status,
        phase: m.phase,
        node_id: m.node_id,
        log: m.log,
        started_at: rfc3339(m.created_at),
        updated_at: rfc3339(m.updated_at),
    }
}

/// An account that was not moved.
#[derive(Debug, Serialize, ToSchema)]
pub struct SkippedAccount {
    /// The account.
    pub tenant_id: Uuid,
    /// Its name on the server.
    pub username: String,
    /// Why.
    pub reason: String,
}

/// What `move-accounts` did.
#[derive(Debug, Serialize, ToSchema)]
pub struct MoveView {
    /// Transfers that were started (follow them under `/api/v1/admin/node-transfers`).
    pub started: Vec<Uuid>,
    /// Accounts that were left where they are.
    pub skipped: Vec<SkippedAccount>,
}

// ----------------------------------------------------------------------------------- checks

fn field(name: &'static str, code: &'static str, message: &'static str) -> AppError {
    AppError::validation(vec![FieldError::new(name, code, message)])
}

/// A host name or an IP address, as the SSH client takes it.
fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with('-')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
}

fn valid_user(user: &str) -> bool {
    let mut chars = user.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && user.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
}

/// `SHA256:` and 43 characters of unpadded base64, as OpenSSH prints a key.
fn valid_host_key(fp: &str) -> bool {
    fp.strip_prefix("SHA256:").is_some_and(|b| {
        b.len() == 43
            && b.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/'))
    })
}

fn check_login(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    host_key: &str,
) -> Result<(), AppError> {
    let mut errors = Vec::new();
    if !valid_host(host) {
        errors.push(FieldError::new(
            "host",
            "invalid_format",
            "Use the server's name or IP address.",
        ));
    }
    if port == 0 {
        errors.push(FieldError::new(
            "port",
            "invalid_format",
            "Use a port from 1 to 65535.",
        ));
    }
    if !valid_user(user) {
        errors.push(FieldError::new(
            "user",
            "invalid_format",
            "Use a plain login name, such as root.",
        ));
    }
    if password.is_empty() || password.len() > 1024 {
        errors.push(FieldError::new(
            "password",
            "invalid_length",
            "Give the server's password (up to 1024 characters).",
        ));
    }
    if !valid_host_key(host_key) {
        errors.push(FieldError::new(
            "host_key_sha256",
            "invalid_format",
            "Give the fingerprint the probe showed (SHA256:...).",
        ));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::validation(errors))
    }
}

/// What the SSH client says, without ever holding the credential.
fn ssh_error(e: &FetchError) -> AppError {
    match e {
        FetchError::Refused(_) => field(
            "host",
            "address_refused",
            "The panel may not connect to this address.",
        ),
        FetchError::Login => AppError::conflict(
            "ssh_login_refused",
            "The server refused the login (check the user and the password, and that password login is allowed).",
        ),
        FetchError::HostKeyChanged { .. } => AppError::conflict(
            "ssh_host_key_changed",
            "The server shows another host key than the one confirmed: the connection was refused.",
        ),
        other => AppError::conflict("ssh_unreachable", other.to_string()),
    }
}

fn source(login: Login) -> Source {
    Source {
        host: login.host,
        port: login.port,
        user: login.user,
        credential: Credential::Password(login.password),
        host_key: Some(login.host_key),
        path: String::new(),
        allow_local: login.allow_local,
    }
}

struct Login {
    host: String,
    port: u16,
    user: String,
    password: SecretString,
    host_key: String,
    allow_local: bool,
}

// ----------------------------------------------------------------------------------- facts

/// Asked of the new server. No single quote inside: it is wrapped in `sh -c '...'`.
const FACTS_SCRIPT: &str = r#". /etc/os-release 2>/dev/null; echo "os=${PRETTY_NAME:-unknown}"; echo "os_id=${ID:-}:${VERSION_ID:-}"; echo "arch=$(uname -m)"; echo "cpus=$(nproc)"; echo "memory_kb=$(grep MemTotal /proc/meminfo | tr -dc 0-9)"; echo "disk_free_gb=$(df -BG --output=avail / | tail -1 | tr -dc 0-9)"; if [ -c /dev/kvm ]; then echo kvm=yes; else echo kvm=no; fi; echo "uid=$(id -u)"; if sudo -n true 2>/dev/null; then echo sudo=yes; else echo sudo=no; fi; if [ "$(stat -fc %T /sys/fs/cgroup 2>/dev/null)" = cgroup2fs ]; then echo cgroup2=yes; else echo cgroup2=no; fi; if command -v systemctl >/dev/null 2>&1; then echo systemd=yes; else echo systemd=no; fi"#;

fn supported_os(id_version: &str) -> bool {
    matches!(
        id_version,
        "ubuntu:22.04" | "ubuntu:24.04" | "ubuntu:26.04" | "debian:12" | "debian:13"
    ) || {
        let (id, version) = id_version.split_once(':').unwrap_or((id_version, ""));
        matches!(id, "almalinux" | "rocky" | "rhel" | "centos" | "ol")
            && (version == "9"
                || version == "10"
                || version.starts_with("9.")
                || version.starts_with("10."))
    }
}

fn parse_facts(output: &[String]) -> ServerFacts {
    let get = |key: &str| -> &str {
        output
            .iter()
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
            .unwrap_or("")
    };
    let number = |key: &str| get(key).trim().parse::<u64>().unwrap_or(0);
    let facts_root = get("uid").trim() == "0";
    let can_sudo = get("sudo") == "yes";
    let mut facts = ServerFacts {
        os: get("os").to_owned(),
        arch: get("arch").to_owned(),
        cpus: u32::try_from(number("cpus")).unwrap_or(0),
        memory_mb: number("memory_kb") / 1024,
        disk_free_gb: number("disk_free_gb"),
        kvm: get("kvm") == "yes",
        root: facts_root,
        problems: Vec::new(),
    };
    let mut no = |text: &str| facts.problems.push(text.to_owned());
    let mut found = Vec::new();
    if !facts_root && !can_sudo {
        found.push("The login is not root and cannot run sudo without a password.");
    }
    if !supported_os(get("os_id")) {
        found.push("This operating system is not supported (Ubuntu 22.04+, Debian 12+, AlmaLinux, Rocky or RHEL 9+).");
    }
    if get("arch") != "x86_64" && get("arch") != "aarch64" {
        found.push("Only x86_64 and aarch64 servers are supported.");
    }
    if number("memory_kb") / 1024 < 1800 {
        found.push("At least 2 GB of memory are needed.");
    }
    if number("disk_free_gb") < 10 {
        found.push("At least 10 GB free on / are needed.");
    }
    if get("cgroup2") != "yes" {
        found.push("cgroup v2 is not in use (hosting accounts' limits would not work).");
    }
    if get("systemd") != "yes" {
        found.push("systemd is required.");
    }
    for text in found {
        no(text);
    }
    facts
}

// ----------------------------------------------------------------------------------- the job

/// The one-command join, as run on the new server (a pipeline from the installer's address). `sudo_prefix`
/// is `sudo -n ` for a login that is not root. The token goes in the installer's environment, not on a
/// command line.
fn remote_command(
    installer_url: &str,
    sudo_prefix: &str,
    token: &str,
    server: &str,
    fingerprint: &str,
) -> String {
    format!(
        "export DEBIAN_FRONTEND=noninteractive; \
         {{ curl -fsSL '{installer_url}' || wget -qO- '{installer_url}'; }} | \
         {sudo_prefix}env BUILTPANEL_ENROLL_TOKEN='{token}' bash -s -- --join-server '{server}' --join-fingerprint 'sha256:{fingerprint}' 2>&1"
    )
}

/// Removes every secret from a line of output.
fn scrub(line: &str, secrets: &[&str]) -> String {
    let mut out = line.to_owned();
    for secret in secrets {
        if !secret.is_empty() {
            out = out.replace(secret, "***");
        }
    }
    out
}

async fn fail(state: &NodesState, id: Uuid, phase: &str, message: &str) {
    let _ = store::append(&state.db, id, &format!("{message}\n"), None).await;
    let _ = store::finish(&state.db, id, false, phase, None).await;
}

async fn run_job(
    state: NodesState,
    id: Uuid,
    account: Uuid,
    login: Login,
    node_name: String,
    server: String,
    fingerprint: String,
) {
    let _ = store::append(&state.db, id, "Connecting.\n", Some("connecting")).await;
    let password = login.password.expose_secret().to_owned();
    let (token_row, token) = match issue_enrollment(&state, account, &node_name).await {
        Ok(issued) => issued,
        Err(e) => {
            fail(
                &state,
                id,
                "failed_token",
                &format!("Cannot prepare the node's enrolment ({}).", e.code()),
            )
            .await;
            return;
        }
    };
    let sudo = if login.user == "root" { "" } else { "sudo -n " };
    let command = remote_command(
        &state.migrate.installer_url,
        sudo,
        &token,
        &server,
        &fingerprint,
    );
    let _ = store::append(
        &state.db,
        id,
        "Installing the node (this takes several minutes).\n",
        Some("installing"),
    )
    .await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let writer = {
        let db = state.db.clone();
        let secrets = [password.clone(), token.clone()];
        tokio::spawn(async move {
            let secrets: Vec<&str> = secrets.iter().map(String::as_str).collect();
            let mut batch = String::new();
            loop {
                let got = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await;
                match got {
                    Ok(Some(line)) => {
                        batch.push_str(&scrub(&line, &secrets));
                        batch.push('\n');
                        if batch.len() < 4096 {
                            continue;
                        }
                    }
                    Ok(None) => {
                        if !batch.is_empty() {
                            let _ = store::append(&db, id, &batch, None).await;
                        }
                        return;
                    }
                    Err(_) => {}
                }
                if !batch.is_empty() {
                    let _ = store::append(&db, id, &batch, None).await;
                    batch.clear();
                }
            }
        })
    };
    let ran = remote::run(source(login), &command, |line| {
        let _ = tx.send(line.to_owned());
    })
    .await;
    drop(tx);
    let _ = writer.await;
    match ran {
        Ok(r) if r.status == Some(0) => {}
        Ok(r) => {
            let status = r
                .status
                .map_or_else(|| "none".to_owned(), |s| s.to_string());
            fail(
                &state,
                id,
                "install_failed",
                &format!("The installer ended with status {status}."),
            )
            .await;
            return;
        }
        Err(e) => {
            fail(
                &state,
                id,
                "install_failed",
                &scrub(&e.to_string(), &[&password, &token]),
            )
            .await;
            return;
        }
    }
    let _ = store::append(
        &state.db,
        id,
        "Waiting for the node to come online.\n",
        Some("joining"),
    )
    .await;
    let _ = token_row;
    let deadline = tokio::time::Instant::now() + JOIN_WAIT;
    loop {
        let node: Result<Option<Uuid>, _> =
            sqlx::query_scalar("SELECT id FROM nodes WHERE name = $1 AND status = 'online'")
                .bind(&node_name)
                .fetch_optional(&state.db)
                .await;
        if let Ok(Some(node_id)) = node {
            let _ = store::append(&state.db, id, "The node is online.\n", None).await;
            let _ = store::finish(&state.db, id, true, "joined", Some(node_id)).await;
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            fail(
                &state,
                id,
                "join_timeout",
                "The node did not come online in time: check that this panel's agent port is reachable from the new server.",
            )
            .await;
            return;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

// ----------------------------------------------------------------------------------- handlers

fn login_of(
    state: &NodesState,
    host: String,
    port: u16,
    user: String,
    password: String,
    host_key: String,
) -> Login {
    Login {
        host,
        port,
        user,
        password: SecretString::from(password),
        host_key,
        allow_local: state.migrate.allow_local_ssh,
    }
}

/// Shows the host key of the new server (nothing is sent to it, no password).
#[utoipa::path(
    post,
    path = "/api/v1/admin/system/migrate/probe",
    tag = "nodes",
    operation_id = "probeMigrationServer",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = ProbeRequest,
    responses(
        (status = 200, description = "The key the server shows", body = ProbeView),
        (status = 409, description = "`ssh_unreachable`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid address, or one the panel may not connect to", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn probe(
    State(state): State<NodesState>,
    RequireRecent(_, _): RequireRecent<NodesManage>,
    Json(req): Json<ProbeRequest>,
) -> Result<Json<ProbeView>, AppError> {
    if !valid_host(&req.host) || req.port == 0 {
        return Err(field(
            "host",
            "invalid_format",
            "Use the server's name or IP address.",
        ));
    }
    let host_key_sha256 =
        remote::probe_host_key(&req.host, req.port, state.migrate.allow_local_ssh)
            .await
            .map_err(|e| ssh_error(&e))?;
    Ok(Json(ProbeView { host_key_sha256 }))
}

/// Logs in to the new server and says whether a node can be installed on it.
#[utoipa::path(
    post,
    path = "/api/v1/admin/system/migrate/check",
    tag = "nodes",
    operation_id = "checkMigrationServer",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = LoginRequest,
    responses(
        (status = 200, description = "What the server is like and what stands in the way", body = ServerFacts),
        (status = 409, description = "`ssh_login_refused`, `ssh_host_key_changed` or `ssh_unreachable`", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid fields", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn check(
    State(state): State<NodesState>,
    RequireRecent(_, _): RequireRecent<NodesManage>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<ServerFacts>, AppError> {
    check_login(
        &req.host,
        req.port,
        &req.user,
        &req.password,
        &req.host_key_sha256,
    )?;
    let login = login_of(
        &state,
        req.host,
        req.port,
        req.user,
        req.password,
        req.host_key_sha256,
    );
    let mut lines = Vec::new();
    let ran = remote::run(source(login), &format!("sh -c '{FACTS_SCRIPT}'"), |l| {
        lines.push(l.to_owned());
    })
    .await
    .map_err(|e| ssh_error(&e))?;
    if ran.status != Some(0) {
        return Err(AppError::conflict(
            "ssh_command_failed",
            "The server did not answer the check.",
        ));
    }
    Ok(Json(parse_facts(&lines)))
}

/// Starts installing a node on the new server and joining it to this panel. Answers at once; follow the job
/// with `GET /api/v1/admin/system/migrate/{id}`.
#[utoipa::path(
    post,
    path = "/api/v1/admin/system/migrate",
    tag = "nodes",
    operation_id = "startServerMigration",
    security(("session" = [])),
    params(("X-CSRF-Token" = String, Header, description = "The session's csrf_token")),
    request_body = StartRequest,
    responses(
        (status = 202, description = "Started", body = ServerMigrationView),
        (status = 409, description = "`migration_running` (one at a time), `node_name_taken`, `license_limit` or an SSH error (`ssh_login_refused`, `ssh_host_key_changed`, `ssh_unreachable`)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid fields", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 503, description = "`enrollment_unavailable`: the panel has no address or CA for nodes to join", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn start(
    State(state): State<NodesState>,
    RequireRecent(who, _): RequireRecent<NodesManage>,
    Json(req): Json<StartRequest>,
) -> Result<(StatusCode, Json<ServerMigrationView>), AppError> {
    check_login(
        &req.host,
        req.port,
        &req.user,
        &req.password,
        &req.host_key_sha256,
    )?;
    let node_name = req.node_name.trim().to_owned();
    if !valid_node_name(&node_name) {
        return Err(field(
            "node_name",
            "invalid_format",
            "Use 2-63 characters: a-z, 0-9 and '-', starting with a letter.",
        ));
    }
    let (Some(server), Some(ca)) = (state.public_url.as_deref(), state.ca.as_deref()) else {
        return Err(AppError::unavailable(
            "enrollment_unavailable",
            "This panel has no public address or node CA configured, so a node cannot join it.",
        ));
    };
    let server = server.trim_end_matches('/').to_owned();
    let fingerprint = ca.fingerprint_sha256();
    store::abandon_stale(&state.db).await.map_err(internal)?;
    let running: i64 =
        sqlx::query_scalar("SELECT count(*) FROM server_migrations WHERE status = 'running'")
            .fetch_one(&state.db)
            .await
            .map_err(internal)?;
    if running > 0 {
        return Err(AppError::conflict(
            "migration_running",
            "Another server is being set up: wait for it to end.",
        ));
    }
    let record = store::insert(
        &state.db,
        &req.host,
        i32::from(req.port),
        &req.user,
        &node_name,
        who.account_id(),
    )
    .await
    .map_err(internal)?;
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("admin.server_migration_started", Outcome::Success)
            .actor(who.account_id())
            .target("server_migration", record.id)
            .metadata(serde_json::json!({
                "host": req.host, "port": req.port, "user": req.user, "node_name": node_name,
            })),
    )
    .await
    .map_err(internal)?;
    let login = login_of(
        &state,
        req.host,
        req.port,
        req.user,
        req.password,
        req.host_key_sha256,
    );
    tokio::spawn(run_job(
        state.clone(),
        record.id,
        who.account_id(),
        login,
        node_name,
        server,
        fingerprint.to_string(),
    ));
    Ok((StatusCode::ACCEPTED, Json(view(record))))
}

/// The newest jobs.
#[utoipa::path(
    get,
    path = "/api/v1/admin/system/migrate",
    tag = "nodes",
    operation_id = "listServerMigrations",
    security(("session" = [])),
    responses(
        (status = 200, description = "The newest first", body = Vec<ServerMigrationView>),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<NodesState>,
    _who: Require<NodesRead>,
) -> Result<Json<Vec<ServerMigrationView>>, AppError> {
    Ok(Json(
        store::list(&state.db, LIST_LIMIT)
            .await
            .map_err(internal)?
            .into_iter()
            .map(view)
            .collect(),
    ))
}

/// One job, with its log.
#[utoipa::path(
    get,
    path = "/api/v1/admin/system/migrate/{id}",
    tag = "nodes",
    operation_id = "getServerMigration",
    security(("session" = [])),
    params(("id" = Uuid, Path, description = "Job id")),
    responses(
        (status = 200, description = "The job", body = ServerMigrationView),
        (status = 404, description = "No such job", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn show(
    State(state): State<NodesState>,
    _who: Require<NodesRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<ServerMigrationView>, AppError> {
    store::get(&state.db, id)
        .await
        .map_err(internal)?
        .map(view)
        .map(Json)
        .ok_or_else(|| AppError::not_found("server migration"))
}

/// Starts moving every running account of one node to the new server (one node transfer each).
#[utoipa::path(
    post,
    path = "/api/v1/admin/system/migrate/{id}/move-accounts",
    tag = "nodes",
    operation_id = "moveAccountsToMigratedServer",
    security(("session" = [])),
    params(
        ("id" = Uuid, Path, description = "Job id (it must have ended with the node online)"),
        ("X-CSRF-Token" = String, Header, description = "The session's csrf_token")
    ),
    request_body = MoveRequest,
    responses(
        (status = 200, description = "Which transfers started and which accounts were left", body = MoveView),
        (status = 404, description = "No such job or node", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`migration_not_done`, `node_unavailable`, `same_node` or `no_backup_destination`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn move_accounts(
    State(state): State<NodesState>,
    RequireRecent(who, _): RequireRecent<NodesManage>,
    Path(id): Path<Uuid>,
    Json(req): Json<MoveRequest>,
) -> Result<Json<MoveView>, AppError> {
    let job = store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("server migration"))?;
    let to_node = match (job.status.as_str(), job.node_id) {
        ("completed", Some(node)) => node,
        _ => {
            return Err(AppError::conflict(
                "migration_not_done",
                "The new server has not joined yet.",
            ));
        }
    };
    if to_node == req.from_node_id {
        return Err(AppError::conflict(
            "same_node",
            "That is the new server itself.",
        ));
    }
    let from_exists: Option<bool> = sqlx::query_scalar("SELECT true FROM nodes WHERE id = $1")
        .bind(req.from_node_id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal)?;
    if from_exists.is_none() {
        return Err(AppError::not_found("node"));
    }
    let online: Option<bool> =
        sqlx::query_scalar("SELECT status = 'online' FROM nodes WHERE id = $1")
            .bind(to_node)
            .fetch_optional(&state.db)
            .await
            .map_err(internal)?;
    if online != Some(true) {
        return Err(AppError::conflict(
            "node_unavailable",
            "The new node is not online.",
        ));
    }
    let destination = backup_store::default_destination(&state.db)
        .await
        .map_err(internal)?
        .ok_or_else(|| {
            AppError::conflict(
                "no_backup_destination",
                "Set a default backup destination first: the accounts are copied through it.",
            )
        })?;
    let tenants: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, username FROM tenants WHERE node_id = $1 AND status = 'active' ORDER BY created_at",
    )
    .bind(req.from_node_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;
    let mut started = Vec::new();
    let mut skipped = Vec::new();
    for (tenant_id, username) in tenants {
        let unsupported = transfers::unsupported(&state.db, tenant_id)
            .await
            .map_err(internal)?;
        if !unsupported.is_empty() {
            skipped.push(SkippedAccount {
                tenant_id,
                username,
                reason: format!(
                    "A transfer cannot carry these yet: {}.",
                    unsupported.join(", ")
                ),
            });
            continue;
        }
        match transfers::create(
            &state.db,
            tenant_id,
            to_node,
            destination,
            req.auto_cutover,
            Some(who.account_id()),
        )
        .await
        {
            Ok(made) => {
                builtpanel_audit::insert(
                    &state.db,
                    &AuditEntry::new("node_transfer.started", Outcome::Success)
                        .actor(who.account_id())
                        .target("node_transfer", made.id)
                        .metadata(serde_json::json!({
                            "tenant_id": tenant_id,
                            "from_node": req.from_node_id,
                            "to_node": to_node,
                            "server_migration": id,
                        })),
                )
                .await
                .map_err(internal)?;
                started.push(made.id);
            }
            Err(CreateError::Busy) => skipped.push(SkippedAccount {
                tenant_id,
                username,
                reason: "This account is being moved already.".to_owned(),
            }),
            Err(CreateError::Database(e)) => return Err(internal(e)),
        }
    }
    builtpanel_audit::insert(
        &state.db,
        &AuditEntry::new("admin.server_migration_accounts_moved", Outcome::Success)
            .actor(who.account_id())
            .target("server_migration", id)
            .metadata(serde_json::json!({
                "from_node": req.from_node_id,
                "started": started.len(),
                "skipped": skipped.len(),
                "auto_cutover": req.auto_cutover,
            })),
    )
    .await
    .map_err(internal)?;
    Ok(Json(MoveView { started, skipped }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    #[test]
    fn what_may_be_typed_is_checked() {
        assert!(
            valid_host("203.0.113.10")
                && valid_host("srv-1.example.com")
                && valid_host("2001:db8::1")
        );
        for bad in [
            "",
            "-oProxyCommand=x",
            "a b",
            "a;b",
            "a/b",
            &"a".repeat(254),
        ] {
            assert!(!valid_host(bad), "{bad:?}");
        }
        assert!(valid_user("root") && valid_user("deploy-1") && valid_user("_apt"));
        for bad in ["", "Root", "a b", "1abc", "a;b", &"a".repeat(33)] {
            assert!(!valid_user(bad), "{bad:?}");
        }
        let fp = format!("SHA256:{}", "A".repeat(43));
        assert!(valid_host_key(&fp));
        assert!(!valid_host_key("SHA256:short") && !valid_host_key(&"A".repeat(43)));
    }

    #[test]
    fn the_command_carries_the_token_in_the_environment_only() {
        let c = remote_command(
            "https://example.com/get.sh",
            "sudo -n ",
            "dhn_tok",
            "https://panel.example.com:8443",
            "ab12",
        );
        assert!(c.contains("sudo -n env BUILTPANEL_ENROLL_TOKEN='dhn_tok' bash -s --"));
        assert!(c.contains(
            "--join-server 'https://panel.example.com:8443' --join-fingerprint 'sha256:ab12'"
        ));
        assert!(c.contains("curl -fsSL 'https://example.com/get.sh' || wget -qO-"));
    }

    #[test]
    fn output_never_keeps_a_secret() {
        let secrets = ["hunter2", "dhn_tok"];
        assert_eq!(
            scrub("login hunter2 token dhn_tok hunter2", &secrets),
            "login *** token *** ***"
        );
        assert_eq!(scrub("plain", &["", "x"]), "plain");
    }

    #[test]
    fn the_server_is_judged_from_what_it_says() {
        let good = parse_facts(&lines(
            "os=Ubuntu 24.04.2 LTS\nos_id=ubuntu:24.04\narch=x86_64\ncpus=4\nmemory_kb=8000000\ndisk_free_gb=80\nkvm=yes\nuid=0\nsudo=no\ncgroup2=yes\nsystemd=yes",
        ));
        assert!(good.problems.is_empty(), "{:?}", good.problems);
        assert!(good.root && good.kvm);
        assert_eq!(
            (good.cpus, good.memory_mb, good.disk_free_gb),
            (4, 7812, 80)
        );
        let bad = parse_facts(&lines(
            "os=CentOS 7\nos_id=centos:7\narch=armv7l\ncpus=1\nmemory_kb=900000\ndisk_free_gb=3\nkvm=no\nuid=1000\nsudo=no\ncgroup2=no\nsystemd=no",
        ));
        assert_eq!(bad.problems.len(), 7, "{:?}", bad.problems);
        let sudoer = parse_facts(&lines(
            "os_id=debian:12\narch=aarch64\nmemory_kb=4000000\ndisk_free_gb=20\nuid=1000\nsudo=yes\ncgroup2=yes\nsystemd=yes",
        ));
        assert!(sudoer.problems.is_empty(), "{:?}", sudoer.problems);
    }
}
