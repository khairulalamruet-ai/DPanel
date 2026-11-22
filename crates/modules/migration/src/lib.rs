//! Bringing an account over from another hosting panel (T15.04, T15.06).
//!
//! | Method | Path | Needs |
//! |---|---|---|
//! | GET, POST | `/api/v1/admin/migration/transfers` | `admin.migration.read`, `admin.migration.manage` |
//! | GET, DELETE | `/api/v1/admin/migration/transfers/{id}` | read, manage |
//! | POST | `/api/v1/admin/migration/transfers/{id}/retry` | manage |
//! | POST | `/api/v1/admin/migration/transfers/{id}/cancel` | manage |
//!
//! A transfer reads a backup file that waits in the panel's import folder
//! (`<imports.spool_dir>/migration/<file>`), works out what importing it would do
//! ([`builtpanel_import::plan`], a pure function: nothing is written) and keeps the answer, the
//! steps and a log. A dry run ends there. Fetching the backup from another server (T15.05) and
//! the real import (the executor) come on top of this record.

use std::{fs::File, io::BufReader, path::PathBuf};

pub mod exec;
pub mod remote;

use axum::{
    Json, Router,
    extract::{FromRef, Path, State},
    http::StatusCode,
    routing::{get, post},
};
use builtpanel_audit::{AuditEntry, Outcome};
use builtpanel_database::{
    migration_transfers::{self as store, NewTransfer, Progress, Transfer},
    sqlx::PgPool,
};
use builtpanel_http_kit::{AppError, FieldError, ProblemDetails};
use builtpanel_import::{Existing, Include, Limits, OnConflict, Options, SourceKind};
use builtpanel_rbac::{
    Principal, Require,
    permissions::{MigrationManage, MigrationRead},
};
use builtpanel_session::{SessionStore, rfc3339};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

/// The module's manifest.
pub const MANIFEST: &str = include_str!("../../../../modules/migration/module.toml");

/// Most transfers listed.
const LIST_LIMIT: i64 = 200;

/// What the module's handlers share.
#[derive(Clone)]
pub struct MigrationState {
    /// Database pool.
    pub db: PgPool,
    /// Session store.
    pub sessions: SessionStore,
    /// The import folder's parent (`imports.spool_dir`); backups wait in its `migration` folder.
    pub spool_dir: PathBuf,
    /// How the executor uses the panel's own API.
    pub loopback: exec::Loopback,
    /// The biggest backup that is fetched from another server, in bytes.
    pub max_backup_bytes: u64,
    /// Whether a server on this machine's own networks may be fetched from (tests only).
    pub allow_local_sources: bool,
}

impl FromRef<MigrationState> for PgPool {
    fn from_ref(s: &MigrationState) -> Self {
        s.db.clone()
    }
}

impl FromRef<MigrationState> for SessionStore {
    fn from_ref(s: &MigrationState) -> Self {
        s.sessions.clone()
    }
}

/// The administrator's routes.
pub fn admin_router() -> Router<MigrationState> {
    Router::new()
        .route("/api/v1/admin/migration/transfers", get(list).post(create))
        .route(
            "/api/v1/admin/migration/transfers/{id}",
            get(show).delete(remove),
        )
        .route("/api/v1/admin/migration/transfers/{id}/retry", post(retry))
        .route(
            "/api/v1/admin/migration/transfers/{id}/cancel",
            post(cancel),
        )
}

fn internal(err: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> AppError {
    AppError::internal(err)
}

fn audit(action: &'static str, who: &Principal, id: Uuid) -> AuditEntry {
    AuditEntry::new(action, Outcome::Success)
        .actor(who.account_id())
        .target("migration_transfer", id)
}

// ------------------------------------------------------------------------- views

/// One step of a transfer.
#[derive(Debug, Serialize, ToSchema)]
pub struct StepView {
    /// Id, `step-<n>`.
    pub id: String,
    /// What it does.
    pub name: String,
    /// `pending`, `running`, `completed` or `failed`.
    pub status: String,
    /// What it did or why it failed.
    pub details: Option<String>,
}

/// A transfer.
#[derive(Debug, Serialize, ToSchema)]
pub struct MigrationTransferView {
    /// Id.
    pub id: Uuid,
    /// `cpanel`, `cyberpanel`, `openpanel` or `builtpanel_archive`.
    pub source_type: String,
    /// The server it was fetched from.
    pub source_host: Option<String>,
    /// The backup's file name.
    pub backup_file: String,
    /// The account's name here.
    pub tenant_username: String,
    /// The main domain here.
    pub target_domain: String,
    /// Whether it only plans.
    pub dry_run: bool,
    /// `pending`, `running`, `completed`, `failed` or `cancelled`.
    pub status: String,
    /// 0 to 100.
    pub progress_percent: i32,
    /// The step being done.
    pub current_step: String,
    /// The steps, in order.
    pub steps: Vec<StepView>,
    /// The log (the last 2000 lines).
    pub logs: Vec<String>,
    /// The backup's size, in bytes.
    pub size_bytes: Option<i64>,
    /// What the import does and what stands in its way (the dry run's answer).
    pub plan: Option<Value>,
    /// Why it failed.
    pub error_message: Option<String>,
    /// RFC 3339, UTC.
    #[schema(format = DateTime)]
    pub created_at: String,
    /// RFC 3339, UTC.
    #[schema(format = DateTime)]
    pub updated_at: String,
}

fn steps_of(v: &Value) -> Vec<StepView> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(StepView {
                        id: s.get("id")?.as_str()?.to_owned(),
                        name: s.get("name")?.as_str()?.to_owned(),
                        status: s.get("status")?.as_str()?.to_owned(),
                        details: s.get("details").and_then(Value::as_str).map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn view(t: Transfer) -> MigrationTransferView {
    MigrationTransferView {
        id: t.id,
        source_type: t.source_type,
        source_host: t.source_host,
        backup_file: t.backup_file,
        tenant_username: t.tenant_username,
        target_domain: t.target_domain,
        dry_run: t.dry_run,
        status: t.status,
        progress_percent: t.progress_percent,
        current_step: t.current_step,
        steps: steps_of(&t.steps),
        logs: t
            .logs
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        size_bytes: t.size_bytes,
        plan: t.plan,
        error_message: t.error_message,
        created_at: rfc3339(t.created_at),
        updated_at: rfc3339(t.updated_at),
    }
}

// --------------------------------------------------------------------- the dry run

/// What a dry run found.
struct Planned {
    plan: builtpanel_import::Plan,
    size: i64,
}

/// A backup file name that stays in the import folder.
fn valid_file_name(name: &str) -> bool {
    (1..=200).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn conflict_policy(text: &str) -> OnConflict {
    match text {
        "rename" => OnConflict::Rename,
        "skip" => OnConflict::Skip,
        _ => OnConflict::Fail,
    }
}

fn kind_name(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Cpanel => "cpanel",
        SourceKind::Cyberpanel => "cyberpanel",
    }
}

/// Reads the backup and plans the import. Reading is blocking work.
async fn dry_run(state: &MigrationState, t: &Transfer) -> Result<Planned, String> {
    let path = state.spool_dir.join("migration").join(&t.backup_file);
    let taken = store::taken(&state.db)
        .await
        .map_err(|e| format!("the panel's names could not be read: {e}"))?;
    let options = Options {
        username: Some(t.tenant_username.clone()),
        main_domain: Some(t.target_domain.clone()),
        on_conflict: conflict_policy(&t.on_conflict),
        include: Include::default(),
        mail_supported: false,
    };
    let source_type = t.source_type.clone();
    tokio::task::spawn_blocking(move || {
        let file = File::open(&path)
            .map_err(|_| "the backup file is not in the import folder".to_owned())?;
        let size = file
            .metadata()
            .map_or(0, |m| i64::try_from(m.len()).unwrap_or(i64::MAX));
        let model = builtpanel_import::read_backup(BufReader::new(file), Limits::default())
            .map_err(|e| e.to_string())?;
        let found = kind_name(model.source.kind);
        if found != source_type {
            return Err(format!("the backup is from {found}, not {source_type}"));
        }
        let existing = Existing {
            usernames: taken.usernames.into_iter().collect(),
            domains: taken.domains.into_iter().collect(),
            db_names: taken.db_names.into_iter().collect(),
        };
        Ok(Planned {
            plan: builtpanel_import::plan(&model, &existing, &options),
            size,
        })
    })
    .await
    .map_err(|e| format!("reading the backup stopped: {e}"))?
}

fn describe(step: &builtpanel_import::Step) -> String {
    use builtpanel_import::Action::{
        AddCronJob, AddDomain, CreateAccount, CreateDatabase, CreateDbUser, CreateDnsZone,
        CreateForwarder, CreateMailbox, ImportDump, RestoreFiles,
    };
    let what = match step.action {
        CreateAccount => "Create the account",
        RestoreFiles => "Restore the files of",
        AddDomain => "Add the domain",
        CreateDatabase => "Create the database",
        ImportDump => "Load the dump into",
        CreateDbUser => "Create the database user",
        CreateDnsZone => "Create the DNS zone",
        AddCronJob => "Add the scheduled job",
        CreateMailbox => "Create the mailbox",
        CreateForwarder => "Create the forwarder",
    };
    match &step.renamed_from {
        Some(old) => format!("{what} {} (was {old})", step.subject),
        None => format!("{what} {}", step.subject),
    }
}

/// Plans a transfer and records the outcome. A dry run ends there; a real import goes on
/// (`true`) when nothing stands in its way.
async fn plan_transfer(state: &MigrationState, t: &Transfer) -> Result<bool, AppError> {
    let started = vec!["Reading the backup...".to_owned()];
    store::append_log(&state.db, t.id, &started)
        .await
        .map_err(internal)?;
    match dry_run(state, t).await {
        Ok(Planned { plan, size }) => {
            let steps: Vec<Value> = plan
                .steps
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    json!({"id": format!("step-{}", i + 1), "name": describe(s),
                           "status": if t.dry_run { "completed" } else { "pending" }})
                })
                .collect();
            let mut log = vec![format!(
                "{} step(s), {} byte(s) of files, {} byte(s) of dumps.",
                plan.steps.len(),
                plan.file_bytes,
                plan.dump_bytes
            )];
            for c in &plan.conflicts {
                log.push(format!(
                    "Conflict: {} `{}` ({:?}).",
                    c.kind, c.name, c.resolution
                ));
            }
            for s in &plan.skipped {
                log.push(format!("Left out: {} `{}`: {}.", s.kind, s.name, s.reason));
            }
            for w in &plan.warnings {
                log.push(format!("Warning: {w:?}"));
            }
            for b in &plan.blockers {
                log.push(format!("Blocked: {b}."));
            }
            log.push(match (plan.ok, t.dry_run) {
                (true, true) => "Dry run finished: nothing was changed.".to_owned(),
                (true, false) => "The plan is fine: the import starts.".to_owned(),
                (false, _) => {
                    "The import cannot run until the problems above are settled.".to_owned()
                }
            });
            store::append_log(&state.db, t.id, &log)
                .await
                .map_err(internal)?;
            let plan_json = serde_json::to_value(&plan).map_err(internal)?;
            let steps_json = Value::Array(steps);
            let go = plan.ok && !t.dry_run;
            let (status, percent, current) = match (plan.ok, t.dry_run) {
                (true, true) => ("completed", 100, "Dry run finished"),
                (true, false) => ("running", 0, "Starting"),
                (false, _) => ("failed", 0, "The plan has problems"),
            };
            store::set_progress(
                &state.db,
                t.id,
                &Progress {
                    status,
                    progress_percent: percent,
                    current_step: current,
                    size_bytes: Some(size),
                    steps: &steps_json,
                    plan: Some(&plan_json),
                    error_message: (!plan.ok)
                        .then_some("The plan has problems that stop the import."),
                },
            )
            .await
            .map_err(internal)?;
            Ok(go)
        }
        Err(why) => {
            store::append_log(&state.db, t.id, &[format!("Failed: {why}")])
                .await
                .map_err(internal)?;
            let none = json!([]);
            store::set_progress(
                &state.db,
                t.id,
                &Progress {
                    status: "failed",
                    progress_percent: 0,
                    current_step: "Reading the backup",
                    size_bytes: None,
                    steps: &none,
                    plan: None,
                    error_message: Some(&why.chars().take(900).collect::<String>()),
                },
            )
            .await
            .map_err(internal)?;
            Ok(false)
        }
    }
}

/// Plans, and for a real import starts the executor in the background.
async fn start(state: &MigrationState, t: &Transfer) -> Result<(), AppError> {
    if plan_transfer(state, t).await? {
        tokio::spawn(exec::run(state.clone(), t.id));
    }
    Ok(())
}

/// Checks the fields that go with `remote_host` and makes the source.
fn remote_source(
    req: &CreateTransfer,
    errors: &mut Vec<FieldError>,
    allow_local: bool,
) -> Option<remote::Source> {
    use secrecy::SecretString;
    let mut bad = |field: &str, code: &'static str, message: &'static str| {
        errors.push(FieldError::new(field, code, message));
    };
    let host = req.remote_host.as_deref().unwrap_or_default().trim();
    if host.is_empty()
        || host.len() > 253
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'_'))
    {
        bad(
            "remote_host",
            "invalid_host",
            "Use a host name or an address.",
        );
    }
    let port = req.remote_ssh_port.unwrap_or(22);
    if port == 0 {
        bad(
            "remote_ssh_port",
            "invalid_port",
            "Use a port from 1 to 65535.",
        );
    }
    let user = req.remote_user.as_deref().unwrap_or_default();
    if user.is_empty()
        || user.len() > 64
        || !user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        bad(
            "remote_user",
            "invalid_user",
            "Use the login name on the server.",
        );
    }
    let path = req.remote_backup_path.as_deref().unwrap_or_default();
    if path.is_empty() || path.len() > 1024 || path.contains('\0') {
        bad(
            "remote_backup_path",
            "invalid_path",
            "Give the backup's path on the server.",
        );
    }
    if let Some(k) = req.host_key_sha256.as_deref()
        && (!k.starts_with("SHA256:") || k.len() > 80)
    {
        bad(
            "host_key_sha256",
            "invalid_fingerprint",
            "Use the fingerprint as SHA256:...",
        );
    }
    let credential = match req.auth_method.as_deref() {
        Some("password") => req
            .password
            .as_deref()
            .filter(|p| !p.is_empty() && p.len() <= 1024)
            .map(|p| remote::Credential::Password(SecretString::from(p.to_owned()))),
        Some("ssh_key") => req
            .ssh_key
            .as_deref()
            .filter(|k| !k.is_empty() && k.len() <= 16 * 1024)
            .map(|k| remote::Credential::Key {
                pem: SecretString::from(k.to_owned()),
                passphrase: req
                    .ssh_key_passphrase
                    .as_deref()
                    .filter(|p| !p.is_empty())
                    .map(|p| SecretString::from(p.to_owned())),
            }),
        _ => None,
    };
    if credential.is_none() {
        bad(
            "auth_method",
            "invalid_login",
            "Use password with a password, or ssh_key with a private key.",
        );
    }
    Some(remote::Source {
        host: host.to_owned(),
        port,
        user: user.to_owned(),
        credential: credential?,
        host_key: req.host_key_sha256.clone(),
        path: path.to_owned(),
        allow_local,
    })
}

/// Fetches the backup of a transfer, then plans (and for a real import, runs) it.
async fn fetch_then_start(state: MigrationState, t: Transfer, source: remote::Source) {
    let dest = state.spool_dir.join("migration").join(&t.backup_file);
    let _ = tokio::fs::create_dir_all(state.spool_dir.join("migration")).await;
    let _ = store::append_log(
        &state.db,
        t.id,
        &[format!(
            "Connecting to {}:{} as {}.",
            source.host, source.port, source.user
        )],
    )
    .await;
    let (db, id) = (state.db.clone(), t.id);
    let progress = move |bytes: u64| {
        let (db, line) = (
            db.clone(),
            format!("Received {} MiB.", bytes / (1024 * 1024)),
        );
        tokio::spawn(async move {
            let _ = store::append_log(&db, id, &[line]).await;
        });
    };
    match remote::fetch(source, &dest, state.max_backup_bytes, progress).await {
        Ok(done) => {
            let _ = store::append_log(
                &state.db,
                t.id,
                &[format!(
                    "Fetched {} byte(s). The server's host key is {}.",
                    done.bytes, done.host_key
                )],
            )
            .await;
            if start(&state, &t).await.is_err() {
                exec::fail_transfer(&state, &t, "the plan could not be made").await;
            }
        }
        Err(e) => exec::fail_transfer(&state, &t, &e.to_string()).await,
    }
}

// ---------------------------------------------------------------------- handlers

/// The transfers, newest first.
#[utoipa::path(
    get, path = "/api/v1/admin/migration/transfers", tag = "migration",
    operation_id = "listMigrationTransfers", security(("session" = [])),
    responses(
        (status = 200, description = "The transfers", body = Vec<MigrationTransferView>),
        (status = 403, description = "`permission_denied`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn list(
    State(state): State<MigrationState>,
    Require(_who, _): Require<MigrationRead>,
) -> Result<Json<Vec<MigrationTransferView>>, AppError> {
    let rows = store::list(&state.db, LIST_LIMIT).await.map_err(internal)?;
    Ok(Json(rows.into_iter().map(view).collect()))
}

/// One transfer, with its steps and log.
#[utoipa::path(
    get, path = "/api/v1/admin/migration/transfers/{id}", tag = "migration",
    operation_id = "getMigrationTransfer", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Transfer id")),
    responses(
        (status = 200, description = "The transfer", body = MigrationTransferView),
        (status = 404, description = "No such transfer", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn show(
    State(state): State<MigrationState>,
    Require(_who, _): Require<MigrationRead>,
    Path(id): Path<Uuid>,
) -> Result<Json<MigrationTransferView>, AppError> {
    let t = store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("transfer"))?;
    Ok(Json(view(t)))
}

/// A transfer to start. The backup is a file in the import folder (`backup_file`) or is fetched
/// from another server over SSH (`remote_host` and what goes with it). The password or key is
/// used for that one connection and is kept nowhere.
#[derive(Deserialize, ToSchema)]
pub struct CreateTransfer {
    /// `cpanel` or `cyberpanel` (`openpanel` and `builtpanel_archive` are not readable yet).
    pub source_type: String,
    /// The backup's file name in the import folder (not with `remote_host`).
    #[serde(default)]
    pub backup_file: Option<String>,
    /// The account's name here.
    pub tenant_username: String,
    /// The main domain here.
    pub target_domain: String,
    /// Only plan (default `true`). With `false` the plan is carried out when nothing stands in
    /// its way.
    #[serde(default = "yes")]
    pub dry_run: bool,
    /// `fail` (default), `rename` or `skip`.
    #[serde(default)]
    pub on_conflict: Option<String>,
    /// Fetch the backup from this server.
    #[serde(default)]
    pub remote_host: Option<String>,
    /// Its SSH port (default 22).
    #[serde(default)]
    pub remote_ssh_port: Option<u16>,
    /// The login name on it.
    #[serde(default)]
    pub remote_user: Option<String>,
    /// `password` or `ssh_key`.
    #[serde(default)]
    pub auth_method: Option<String>,
    /// The password (`auth_method` `password`).
    #[serde(default)]
    #[schema(format = Password)]
    pub password: Option<String>,
    /// The private key (`auth_method` `ssh_key`).
    #[serde(default)]
    #[schema(format = Password)]
    pub ssh_key: Option<String>,
    /// The key's passphrase, when it has one.
    #[serde(default)]
    #[schema(format = Password)]
    pub ssh_key_passphrase: Option<String>,
    /// The backup's path on the server.
    #[serde(default)]
    pub remote_backup_path: Option<String>,
    /// The server's host key fingerprint (`SHA256:...`). Without it nothing is sent to the
    /// server: the transfer fails and says which fingerprint the server showed.
    #[serde(default)]
    pub host_key_sha256: Option<String>,
}

fn yes() -> bool {
    true
}

/// Start a transfer: read the backup and plan the import.
#[utoipa::path(
    post, path = "/api/v1/admin/migration/transfers", tag = "migration",
    operation_id = "createMigrationTransfer", security(("session" = [])),
    request_body = CreateTransfer,
    responses(
        (status = 201, description = "Created and, for a dry run, finished", body = MigrationTransferView),
        (status = 409, description = "`import_unavailable`: this panel cannot run imports", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Invalid input", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn create(
    State(state): State<MigrationState>,
    Require(who, _): Require<MigrationManage>,
    Json(req): Json<CreateTransfer>,
) -> Result<(StatusCode, Json<MigrationTransferView>), AppError> {
    let mut errors = Vec::new();
    if !matches!(req.source_type.as_str(), "cpanel" | "cyberpanel") {
        errors.push(FieldError::new(
            "source_type",
            "unsupported_source",
            "Only cpanel and cyberpanel backups can be read.",
        ));
    }
    let remote = if req.remote_host.is_some() {
        remote_source(&req, &mut errors, state.allow_local_sources)
    } else {
        match req.backup_file.as_deref() {
            Some(f) if valid_file_name(f) => {}
            _ => errors.push(FieldError::new(
                "backup_file",
                "invalid_file_name",
                "Use the file's name in the import folder (letters, digits, . _ -).",
            )),
        }
        None
    };
    if builtpanel_domain::TenantUsername::parse(&req.tenant_username.to_ascii_lowercase()).is_err()
    {
        errors.push(FieldError::new(
            "tenant_username",
            "invalid_username",
            "Use 3 to 32 lower-case letters and digits, starting with a letter.",
        ));
    }
    if builtpanel_domain::Fqdn::parse(&req.target_domain.to_ascii_lowercase()).is_err() {
        errors.push(FieldError::new(
            "target_domain",
            "invalid_domain",
            "Not a valid domain name.",
        ));
    }
    let policy = req.on_conflict.as_deref().unwrap_or("fail");
    if !matches!(policy, "fail" | "rename" | "skip") {
        errors.push(FieldError::new(
            "on_conflict",
            "invalid_policy",
            "Use fail, rename or skip.",
        ));
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    if !req.dry_run && state.loopback.0.get().is_none() {
        return Err(AppError::conflict(
            "import_unavailable",
            "This panel cannot run imports.",
        ));
    }
    let fetched_name = format!("remote-{}.tar.gz", Uuid::new_v4().simple());
    let backup_file = match &remote {
        Some(_) => fetched_name.as_str(),
        None => req.backup_file.as_deref().unwrap_or_default(),
    };
    let t = store::insert(
        &state.db,
        &NewTransfer {
            source_type: &req.source_type,
            source_host: req.remote_host.as_deref(),
            backup_file,
            tenant_username: &req.tenant_username.to_ascii_lowercase(),
            target_domain: &req.target_domain.to_ascii_lowercase(),
            dry_run: req.dry_run,
            on_conflict: policy,
            created_by: who.account_id(),
        },
    )
    .await
    .map_err(internal)?;
    builtpanel_audit::insert(&state.db, &audit("migration.transfer_started", &who, t.id))
        .await
        .map_err(internal)?;
    match remote {
        // The fetch takes as long as the backup is big: it goes on in the background, and then
        // the plan is made. The login lives in that task only.
        Some(source) => {
            let progress = Progress {
                status: "running",
                progress_percent: 0,
                current_step: "Fetching the backup",
                size_bytes: None,
                steps: &json!([]),
                plan: None,
                error_message: None,
            };
            store::set_progress(&state.db, t.id, &progress)
                .await
                .map_err(internal)?;
            tokio::spawn(fetch_then_start(state.clone(), t.clone(), source));
        }
        None => start(&state, &t).await?,
    }
    let done = store::get(&state.db, t.id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("transfer"))?;
    Ok((StatusCode::CREATED, Json(view(done))))
}

/// Run a failed or cancelled transfer again from its first step.
#[utoipa::path(
    post, path = "/api/v1/admin/migration/transfers/{id}/retry", tag = "migration",
    operation_id = "retryMigrationTransfer", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Transfer id")),
    responses(
        (status = 200, description = "Run again", body = MigrationTransferView),
        (status = 404, description = "No such transfer", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`not_retryable`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn retry(
    State(state): State<MigrationState>,
    Require(who, _): Require<MigrationManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<MigrationTransferView>, AppError> {
    let existing = store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("transfer"))?;
    // The login of a transfer from a server was kept nowhere: without the file it cannot be
    // fetched again.
    if existing.source_host.is_some()
        && !state
            .spool_dir
            .join("migration")
            .join(&existing.backup_file)
            .exists()
    {
        return Err(AppError::conflict(
            "login_needed",
            "The backup was not fetched and the login was not kept: start a new transfer.",
        ));
    }
    // A real import that got as far as its steps goes on from the step that stopped it.
    let resumed = !existing.dry_run
        && state.loopback.0.get().is_some()
        && store::resume(&state.db, id).await.map_err(internal)?;
    if resumed {
        builtpanel_audit::insert(&state.db, &audit("migration.transfer_retried", &who, id))
            .await
            .map_err(internal)?;
        store::append_log(
            &state.db,
            id,
            &["Going on from the step that stopped it.".to_owned()],
        )
        .await
        .map_err(internal)?;
        tokio::spawn(exec::run(state.clone(), id));
    } else {
        if !store::requeue(&state.db, id).await.map_err(internal)? {
            return Err(AppError::conflict(
                "not_retryable",
                "Only a transfer that failed or was cancelled can be run again.",
            ));
        }
        let t = store::get(&state.db, id)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::not_found("transfer"))?;
        builtpanel_audit::insert(&state.db, &audit("migration.transfer_retried", &who, id))
            .await
            .map_err(internal)?;
        store::append_log(
            &state.db,
            id,
            &["Running again from the first step.".to_owned()],
        )
        .await
        .map_err(internal)?;
        start(&state, &t).await?;
    }
    let done = store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("transfer"))?;
    Ok(Json(view(done)))
}

/// Stop a transfer that has not ended.
#[utoipa::path(
    post, path = "/api/v1/admin/migration/transfers/{id}/cancel", tag = "migration",
    operation_id = "cancelMigrationTransfer", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Transfer id")),
    responses(
        (status = 200, description = "Cancelled", body = MigrationTransferView),
        (status = 404, description = "No such transfer", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`not_cancellable`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn cancel(
    State(state): State<MigrationState>,
    Require(who, _): Require<MigrationManage>,
    Path(id): Path<Uuid>,
) -> Result<Json<MigrationTransferView>, AppError> {
    store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("transfer"))?;
    if !store::cancel(&state.db, id).await.map_err(internal)? {
        return Err(AppError::conflict(
            "not_cancellable",
            "This transfer has already ended.",
        ));
    }
    builtpanel_audit::insert(&state.db, &audit("migration.transfer_cancelled", &who, id))
        .await
        .map_err(internal)?;
    let t = store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("transfer"))?;
    Ok(Json(view(t)))
}

/// Delete a transfer that has ended.
#[utoipa::path(
    delete, path = "/api/v1/admin/migration/transfers/{id}", tag = "migration",
    operation_id = "deleteMigrationTransfer", security(("session" = [])),
    params(("id" = Uuid, Path, description = "Transfer id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such transfer", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "`still_running`", body = ProblemDetails, content_type = "application/problem+json")
    )
)]
pub async fn remove(
    State(state): State<MigrationState>,
    Require(who, _): Require<MigrationManage>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    store::get(&state.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| AppError::not_found("transfer"))?;
    if !store::delete(&state.db, id).await.map_err(internal)? {
        return Err(AppError::conflict(
            "still_running",
            "Cancel the transfer before deleting it.",
        ));
    }
    builtpanel_audit::insert(&state.db, &audit("migration.transfer_deleted", &who, id))
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// OpenAPI description of the module.
#[derive(OpenApi)]
#[openapi(
    paths(list, show, create, retry, cancel, remove),
    components(schemas(MigrationTransferView, StepView, CreateTransfer)),
    tags((name = "migration", description = "Bring an account over from another hosting panel"))
)]
pub struct MigrationApi;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_stay_in_the_folder() {
        assert!(valid_file_name("cpmove-alice.tar.gz"));
        assert!(!valid_file_name("../etc/passwd"));
        assert!(!valid_file_name("a/b.tar"));
        assert!(!valid_file_name(".hidden"));
        assert!(!valid_file_name("a..b"));
        assert!(!valid_file_name(""));
    }
}
